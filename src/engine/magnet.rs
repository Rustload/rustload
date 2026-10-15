//! Magnet URIs: display name, BitTorrent info-hash and size, read without contacting anyone.

use percent_encoding::percent_decode_str;

const SCHEME: &str = "magnet:?";

#[derive(Debug, Default, PartialEq)]
pub struct Magnet {
    /// `dn`: display name.
    pub name: Option<String>,
    /// BitTorrent v1 info-hash of `xt=urn:btih:`, lowercase hex.
    pub info_hash: Option<String>,
    /// `xl`: exact length in bytes.
    pub size: Option<u64>,
}

pub fn is_magnet(link: &str) -> bool {
    link.get(..SCHEME.len()).is_some_and(|scheme| scheme.eq_ignore_ascii_case(SCHEME))
}

/// Reads the parameters of a magnet URI; `None` if `uri` is not one. The first value of each
/// parameter wins (`xt.1`, `xt.2`... count as `xt`).
pub fn parse(uri: &str) -> Option<Magnet> {
    if !is_magnet(uri) {
        return None;
    }
    let mut magnet = Magnet::default();
    for pair in uri[SCHEME.len()..].split('&') {
        let Some((key, value)) = pair.split_once('=') else { continue };
        let value = percent_decode_str(&value.replace('+', " ")).decode_utf8_lossy().into_owned();
        match key {
            "dn" if magnet.name.is_none() => magnet.name = Some(value).filter(|name| !name.trim().is_empty()),
            "xl" if magnet.size.is_none() => magnet.size = value.parse().ok(),
            _ if (key == "xt" || key.starts_with("xt.")) && magnet.info_hash.is_none() => {
                magnet.info_hash = btih(&value);
            }
            _ => {}
        }
    }
    Some(magnet)
}

/// `urn:btih:<hash>` with a 40-character hex or 32-character base32 hash, as lowercase hex.
fn btih(xt: &str) -> Option<String> {
    const PREFIX: &str = "urn:btih:";
    let hash = xt.get(..PREFIX.len()).filter(|p| p.eq_ignore_ascii_case(PREFIX)).map(|_| &xt[PREFIX.len()..])?;
    match hash.len() {
        40 if hash.chars().all(|c| c.is_ascii_hexdigit()) => Some(hash.to_ascii_lowercase()),
        32 => base32_decode(hash).map(hex::encode),
        _ => None,
    }
}

/// RFC 4648 base32 without padding, case-insensitive.
fn base32_decode(text: &str) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(text.len() * 5 / 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in text.bytes() {
        let value = match byte.to_ascii_uppercase() {
            letter @ b'A'..=b'Z' => letter - b'A',
            digit @ b'2'..=b'7' => digit - b'2' + 26,
            _ => return None,
        };
        buffer = (buffer << 5) | u32::from(value);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";

    #[test]
    fn reads_name_hash_and_size() {
        let magnet =
            parse(&format!("magnet:?xt=urn:btih:{}&dn=My+Show%20S01&xl=1234&tr=udp%3A%2F%2Ft", HEX.to_uppercase()))
                .unwrap();
        assert_eq!(magnet, Magnet { name: Some("My Show S01".into()), info_hash: Some(HEX.into()), size: Some(1234) });
    }

    #[test]
    fn decodes_base32_hashes() {
        let magnet = parse("MAGNET:?xt=urn:btih:YEX6DQDLXISUVHOJ6UM3GNNKPQJWPKEK").unwrap();
        assert_eq!(magnet.info_hash.as_deref(), Some(HEX));
        let lower = parse("magnet:?xt.1=URN:BTIH:yex6dqdlxisuvhoj6um3gnnkpqjwpkek").unwrap();
        assert_eq!(lower.info_hash.as_deref(), Some(HEX));
    }

    #[test]
    fn tolerates_incomplete_magnets() {
        assert_eq!(parse("magnet:?xt=urn:btih:abc&dn="), Some(Magnet::default()));
        assert_eq!(parse("magnet:?xt=urn:btmh:1220abcd").unwrap().info_hash, None);
        assert_eq!(parse("magnet:?xt=urn:btih:YEX6DQDLXISUVHOJ6UM3GNNKPQJWPKE1").unwrap().info_hash, None);
        assert_eq!(parse("https://example.com/?xt=urn:btih:abc"), None);
        assert!(is_magnet("Magnet:?dn=x") && !is_magnet("magnet"));
    }
}
