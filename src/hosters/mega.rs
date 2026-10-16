//! MEGA files and folders.
//!
//! Content is AES-128-CTR encrypted with a key that travels in the URL fragment and never
//! reaches MEGA. CTR is seekable, so each segment is decrypted at its own offset while it is
//! written: multi-connection downloads and resume work as for any other file.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use aes::{
    Aes128,
    cipher::{Array, BlockCipherDecrypt, KeyInit},
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use cbc::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::NoPadding};
use ctr::cipher::{StreamCipher, StreamCipherSeek};
use serde_json::{Value, json};
use url::Url;

use super::{Decryptor, FolderEntry, FolderListing, Hoster, HosterError, HosterLink, host_is};

const API: &str = "https://g.api.mega.co.nz/cs";

pub struct Mega;

#[derive(Debug, PartialEq)]
enum Link {
    File { handle: String, key: Vec<u8> },
    Folder { handle: String, key: Vec<u8>, node: Option<String> },
}

/// Accepts `/file/<h>#<key>`, `/folder/<h>#<key>[/file/<node>]` and the legacy `#!h!key` / `#F!h!key`.
fn parse_link(url: &Url) -> Option<Link> {
    let fragment = url.fragment().unwrap_or_default();
    let mut segments = url.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect::<Vec<_>>()).unwrap_or_default();
    if segments.is_empty() {
        // Legacy links keep everything in the fragment.
        let mut parts = fragment.split('!');
        let kind = parts.next()?;
        let (handle, key) = (parts.next()?, parts.next()?);
        return match kind {
            "" => Some(Link::File { handle: handle.into(), key: decode(key)? }),
            "F" => Some(Link::Folder { handle: handle.into(), key: decode(key)?, node: None }),
            _ => None,
        };
    }
    let handle = segments.get(1)?.to_string();
    match segments.remove(0) {
        "file" => Some(Link::File { handle, key: decode(fragment)? }),
        "folder" => {
            let mut parts = fragment.split('/');
            let key = decode(parts.next()?)?;
            let node = match (parts.next(), parts.next()) {
                (Some("file"), Some(node)) => Some(node.to_owned()),
                _ => None,
            };
            Some(Link::Folder { handle, key, node })
        }
        _ => None,
    }
}

fn decode(text: &str) -> Option<Vec<u8>> {
    let normalized: String = text
        .trim_end_matches('=')
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            c => c,
        })
        .collect();
    URL_SAFE_NO_PAD.decode(normalized).ok()
}

/// File keys are 32 bytes: the AES key is the XOR of both halves, bytes 16..24 are the CTR nonce.
fn split_file_key(key: &[u8]) -> Option<([u8; 16], [u8; 8])> {
    if key.len() != 32 {
        return None;
    }
    let mut aes_key = [0u8; 16];
    for (i, byte) in aes_key.iter_mut().enumerate() {
        *byte = key[i] ^ key[i + 16];
    }
    Some((aes_key, key[16..24].try_into().ok()?))
}

/// Node keys of a shared folder are encrypted with the folder key (AES-ECB).
fn decrypt_node_key(encrypted: &[u8], folder_key: &[u8]) -> Option<Vec<u8>> {
    let cipher = Aes128::new_from_slice(folder_key).ok()?;
    if !encrypted.len().is_multiple_of(16) {
        return None;
    }
    let mut out = encrypted.to_vec();
    for block in out.chunks_mut(16) {
        cipher.decrypt_block(Array::slice_as_mut_array(block)?);
    }
    Some(out)
}

/// Attributes are `MEGA{"n":"name",...}`, AES-CBC encrypted with a zero IV.
fn decrypt_name(attributes: &str, key: &[u8; 16]) -> Option<String> {
    let mut data = decode(attributes)?;
    if data.is_empty() || data.len() % 16 != 0 {
        return None;
    }
    let plain =
        cbc::Decryptor::<Aes128>::new(key.into(), &[0u8; 16].into()).decrypt_padded::<NoPadding>(&mut data).ok()?;
    let text = std::str::from_utf8(plain).ok()?.trim_end_matches('\0');
    let json: Value = serde_json::from_str(text.strip_prefix("MEGA")?).ok()?;
    json["n"].as_str().map(str::to_owned)
}

struct CtrDecryptor {
    key: [u8; 16],
    nonce: [u8; 8],
}

impl Decryptor for CtrDecryptor {
    fn apply(&self, offset: u64, data: &mut [u8]) {
        let mut iv = [0u8; 16];
        iv[..8].copy_from_slice(&self.nonce);
        let mut cipher = ctr::Ctr128BE::<Aes128>::new(&self.key.into(), &iv.into());
        cipher.seek(offset);
        cipher.apply_keystream(data);
    }
}

/// Calls the MEGA API; errors are negative numbers instead of objects.
async fn api(http: &reqwest::Client, request: Value, folder: Option<&str>) -> Result<Value, HosterError> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let id = SEQUENCE.fetch_add(1, Ordering::Relaxed) + rand::random::<u32>() as u64;
    let mut query = vec![("id", id.to_string())];
    if let Some(folder) = folder {
        query.push(("n", folder.to_owned()));
    }
    let response = http.post(API).query(&query).json(&json!([request])).send().await?;
    if !response.status().is_success() {
        return Err(super::status_error(response.status()));
    }
    let body: Value = response.json().await?;
    let result = match &body {
        Value::Array(items) => items.first().cloned().unwrap_or(Value::Null),
        other => other.clone(),
    };
    match result.as_i64() {
        Some(code) if code < 0 => Err(map_error(code)),
        _ => Ok(result),
    }
}

fn map_error(code: i64) -> HosterError {
    match code {
        -9 | -16 | -21 => HosterError::Offline(format!("MEGA error {code} (removed or blocked)")),
        -11 => HosterError::Failed("access denied by MEGA".into()),
        -17 | -18 | -3 | -4 | -6 | -13 | -24 => {
            HosterError::Temporary(format!("MEGA is busy or over quota (error {code})"))
        }
        _ => HosterError::Failed(format!("MEGA error {code}")),
    }
}

struct Node {
    handle: String,
    parent: String,
    folder: bool,
    name: String,
    size: Option<u64>,
}

/// Lists a shared folder with decrypted names. Returns the nodes and the file keys.
async fn list_folder(
    http: &reqwest::Client,
    handle: &str,
    folder_key: &[u8],
) -> Result<Vec<(Node, Vec<u8>)>, HosterError> {
    let listing = api(http, json!({ "a": "f", "c": 1, "r": 1, "ca": 1 }), Some(handle)).await?;
    let mut nodes = Vec::new();
    for item in listing["f"].as_array().into_iter().flatten() {
        let (Some(node_handle), Some(encrypted)) = (item["h"].as_str(), item["k"].as_str()) else { continue };
        // `k` is `owner:key[/owner2:key2]`: any entry decrypts with the shared folder key.
        let Some(key) = encrypted.split('/').find_map(|part| part.split_once(':').map(|(_, k)| k)).and_then(decode)
        else {
            continue;
        };
        let Some(key) = decrypt_node_key(&key, folder_key) else { continue };
        let folder = item["t"].as_i64() == Some(1);
        let attribute_key =
            if folder { key.get(..16).and_then(|k| k.try_into().ok()) } else { split_file_key(&key).map(|(k, _)| k) };
        let name = attribute_key
            .and_then(|k| decrypt_name(item["a"].as_str().unwrap_or_default(), &k))
            .unwrap_or_else(|| node_handle.to_owned());
        let node = Node {
            handle: node_handle.to_owned(),
            parent: item["p"].as_str().unwrap_or_default().to_owned(),
            folder,
            name,
            size: item["s"].as_u64(),
        };
        nodes.push((node, key));
    }
    Ok(nodes)
}

/// `/`-separated path of a node's parent folders below the shared root.
fn parent_path(nodes: &[(Node, Vec<u8>)], node: &Node) -> String {
    let mut parts = Vec::new();
    let mut parent = node.parent.as_str();
    for _ in 0..64 {
        let Some((folder, _)) = nodes.iter().find(|(n, _)| n.handle == parent && n.folder) else { break };
        parts.push(folder.name.as_str());
        parent = folder.parent.as_str();
    }
    parts.reverse();
    // The first element is the shared folder itself (the package folder plays its role).
    parts.into_iter().skip(1).collect::<Vec<_>>().join("/")
}

#[async_trait]
impl Hoster for Mega {
    fn name(&self) -> &'static str {
        "mega"
    }

    fn label(&self) -> &'static str {
        "MEGA"
    }

    fn matches(&self, url: &Url) -> bool {
        host_is(url, &["mega.nz", "mega.co.nz", "mega.io"])
    }

    async fn expand(&self, http: &reqwest::Client, url: &Url) -> Result<Option<FolderListing>, HosterError> {
        let Some(Link::Folder { handle, key, node: None }) = parse_link(url) else { return Ok(None) };
        let nodes = list_folder(http, &handle, &key).await?;
        let root_name = nodes
            .iter()
            .find(|(n, _)| n.folder && !nodes.iter().any(|(p, _)| p.handle == n.parent))
            .map(|(n, _)| n.name.clone());
        let fragment_key = URL_SAFE_NO_PAD.encode(&key);
        let entries = nodes
            .iter()
            .filter(|(n, _)| !n.folder)
            .map(|(n, _)| FolderEntry {
                url: format!("https://mega.nz/folder/{handle}#{fragment_key}/file/{}", n.handle),
                name: Some(n.name.clone()),
                size: n.size,
                rel_dir: parent_path(&nodes, n),
            })
            .collect();
        Ok(Some(FolderListing { name: root_name, entries }))
    }

    async fn resolve(&self, http: &reqwest::Client, url: &Url) -> Result<HosterLink, HosterError> {
        let invalid = || HosterError::Failed("invalid MEGA link (the key after # is required)".into());
        let (file_key, data) = match parse_link(url).ok_or_else(invalid)? {
            Link::File { handle, key } => {
                let data = api(http, json!({ "a": "g", "g": 1, "ssl": 1, "p": handle }), None).await?;
                (key, data)
            }
            Link::Folder { handle, key, node: Some(node) } => {
                let nodes = list_folder(http, &handle, &key).await?;
                let (_, node_key) = nodes
                    .into_iter()
                    .find(|(n, _)| n.handle == node)
                    .ok_or_else(|| HosterError::Offline("file not found in the MEGA folder".into()))?;
                let data = api(http, json!({ "a": "g", "g": 1, "ssl": 1, "n": node }), Some(&handle)).await?;
                (node_key, data)
            }
            Link::Folder { .. } => return Err(HosterError::Failed("MEGA folder links are expanded when added".into())),
        };
        let (aes_key, nonce) = split_file_key(&file_key).ok_or_else(invalid)?;
        let download =
            data["g"].as_str().ok_or_else(|| HosterError::Temporary("MEGA returned no download URL".into()))?;
        Ok(HosterLink {
            filename: decrypt_name(data["at"].as_str().unwrap_or_default(), &aes_key),
            size: data["s"].as_u64(),
            decryptor: Some(Arc::new(CtrDecryptor { key: aes_key, nonce })),
            ..HosterLink::direct(download)
        })
    }
}

#[cfg(test)]
mod tests {
    use cbc::cipher::{BlockModeEncrypt, block_padding::ZeroPadding};

    use super::*;

    #[test]
    fn parses_links() {
        let key = URL_SAFE_NO_PAD.encode([7u8; 32]);
        let file = Url::parse(&format!("https://mega.nz/file/AbCd1234#{key}")).unwrap();
        assert_eq!(parse_link(&file), Some(Link::File { handle: "AbCd1234".into(), key: vec![7; 32] }));
        let legacy = Url::parse(&format!("https://mega.nz/#!AbCd1234!{key}")).unwrap();
        assert_eq!(parse_link(&legacy), Some(Link::File { handle: "AbCd1234".into(), key: vec![7; 32] }));

        let folder_key = URL_SAFE_NO_PAD.encode([1u8; 16]);
        let folder = Url::parse(&format!("https://mega.nz/folder/F00d#{folder_key}/file/N0de")).unwrap();
        assert_eq!(
            parse_link(&folder),
            Some(Link::Folder { handle: "F00d".into(), key: vec![1; 16], node: Some("N0de".into()) })
        );
    }

    #[test]
    fn decryption_is_seekable() {
        let decryptor = CtrDecryptor { key: [3; 16], nonce: [9; 8] };
        let plain: Vec<u8> = (0..10_000).map(|i| (i % 251) as u8).collect();
        let mut whole = plain.clone();
        decryptor.apply(0, &mut whole);
        // Decrypting arbitrary, unaligned pieces gives the same result.
        let mut pieces = plain.clone();
        for (start, end) in [(0, 17), (17, 4096), (4096, 4100), (4100, 10_000)] {
            decryptor.apply(start as u64, &mut pieces[start..end]);
        }
        assert_eq!(whole, pieces);
        decryptor.apply(0, &mut whole);
        assert_eq!(whole, plain);
    }

    /// Known answers computed with another AES implementation (pyca/cryptography).
    #[test]
    fn matches_known_vectors() {
        // Content: AES-128-CTR, the nonce followed by a 64-bit block counter, from any offset.
        let decryptor = CtrDecryptor { key: std::array::from_fn(|i| i as u8), nonce: [9, 8, 7, 6, 5, 4, 3, 2] };
        let mut data: Vec<u8> = (0..40).collect();
        decryptor.apply(0, &mut data);
        assert_eq!(
            hex::encode(&data),
            "fd5e05b0cd34f425574117021da88076838cf0d4848233558cf47459ac3749115a7f65511be8d5f8"
        );
        let mut tail: Vec<u8> = (21..40).collect();
        decryptor.apply(21, &mut tail);
        assert_eq!(tail, data[21..]);

        // Node keys: AES-128-ECB with the folder key.
        let encrypted = hex::decode("b06d7a15c3833e71e9376380a45138359245e136f70ac16343f28e348002f4bb").unwrap();
        assert_eq!(decrypt_node_key(&encrypted, &[0x11; 16]), Some((100..132).collect()));

        // Attributes: AES-128-CBC with a zero IV.
        let key: [u8; 16] = std::array::from_fn(|i| 16 + i as u8);
        let name = decrypt_name("qjYw3t7WFTlOxlJBGY5WEFGNuLFNfWkJw5v5hUEBbXQ", &key);
        assert_eq!(name.as_deref(), Some("Sintel.2010.mkv"));
    }

    #[test]
    fn decrypts_attributes_and_keys() {
        let full_key: Vec<u8> = (0..32).collect();
        let (aes_key, nonce) = split_file_key(&full_key).unwrap();
        assert_eq!(aes_key[0], 16);
        assert_eq!(nonce, [16, 17, 18, 19, 20, 21, 22, 23]);

        let attributes = br#"MEGA{"n":"Movie.mkv"}"#;
        let mut buffer = vec![0u8; attributes.len().div_ceil(16) * 16];
        let encrypted = cbc::Encryptor::<Aes128>::new(&aes_key.into(), &[0u8; 16].into())
            .encrypt_padded_b2b::<ZeroPadding>(attributes, &mut buffer)
            .unwrap();
        assert_eq!(decrypt_name(&URL_SAFE_NO_PAD.encode(encrypted), &aes_key).as_deref(), Some("Movie.mkv"));
    }
}
