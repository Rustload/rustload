//! Small helpers shared across modules.

use std::borrow::Cow;

use sha2::{Digest, Sha256};

/// Current unix time in seconds.
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Random 256-bit token, hex encoded.
pub fn random_token() -> String {
    let bytes: [u8; 32] = rand::random();
    hex::encode(bytes)
}

pub fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

/// `abcd••••wxyz` — enough to recognise a secret without disclosing it.
pub fn mask_secret(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 8 {
        return "••••".into();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}••••{tail}")
}

/// Host part of a URL without a leading `www.`, used for display and host matching.
pub fn url_host(raw: &str) -> Option<String> {
    let host = url::Url::parse(raw).ok()?.host_str()?.to_ascii_lowercase();
    Some(host.strip_prefix("www.").map(str::to_owned).unwrap_or(host))
}

/// `cdn.example.com` → `cdn.example.com`, `example.com`: a host and the domains it belongs to,
/// top-level domain excluded.
pub fn parent_domains(host: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(host), |domain| domain.split_once('.').map(|(_, parent)| parent))
        .take_while(move |domain| domain.contains('.') || *domain == host)
}

/// Whether `host` is `domain` or one of its sub-domains.
pub fn host_matches(host: &str, domain: &str) -> bool {
    parent_domains(host).any(|candidate| candidate == domain)
}

/// `prefix/name`, or `name` alone when `prefix` is empty (relative `/`-separated paths).
pub fn join_relative(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_owned() } else { format!("{prefix}/{name}") }
}

/// Decodes HTML character references (`&amp;`, `&#38;`, `&#x26;`...); unknown ones are kept as is.
pub fn decode_entities(text: &str) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('&') {
        decoded.push_str(&rest[..index]);
        rest = &rest[index..];
        match entity(rest) {
            Some((character, length)) => {
                decoded.push(character);
                rest = &rest[length..];
            }
            None => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    Cow::Owned(decoded)
}

/// The character referenced at the start of `text` (which starts with `&`) and the length of
/// the reference.
fn entity(text: &str) -> Option<(char, usize)> {
    let end = text.char_indices().take(12).find(|(_, c)| *c == ';')?.0;
    let character = match &text[1..end] {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        name => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)?
        }
    };
    Some((character, end + 1))
}

/// Human-readable byte size for log messages (`1.4 GiB`).
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_secrets() {
        assert_eq!(mask_secret("abcdefghijkl"), "abcd••••ijkl");
        assert_eq!(mask_secret("short"), "••••");
    }

    #[test]
    fn extracts_host() {
        assert_eq!(url_host("https://www.1fichier.com/?abc").as_deref(), Some("1fichier.com"));
        assert_eq!(url_host("not a url"), None);
    }

    #[test]
    fn matches_domains_and_subdomains() {
        assert_eq!(
            parent_domains("a.cdn.example.com").collect::<Vec<_>>(),
            ["a.cdn.example.com", "cdn.example.com", "example.com"]
        );
        assert!(host_matches("example.com", "example.com"));
        assert!(host_matches("www.example.com", "example.com"));
        assert!(!host_matches("myexample.com", "example.com"));
        assert!(!host_matches("example.com", "www.example.com"));
        assert!(host_matches("localhost", "localhost"));
    }

    #[test]
    fn joins_relative_paths() {
        assert_eq!(join_relative("", "a.mkv"), "a.mkv");
        assert_eq!(join_relative("Show/S01", "a.mkv"), "Show/S01/a.mkv");
    }

    #[test]
    fn decodes_character_references() {
        assert_eq!(decode_entities("a&amp;b&#x26;c&#38;d&lt;&quot;&apos;&#39;"), "a&b&c&d<\"''");
        assert_eq!(decode_entities("R&D &unknown; &#xZZ; &"), "R&D &unknown; &#xZZ; &");
        assert!(matches!(decode_entities("plain"), Cow::Borrowed("plain")));
    }

    #[test]
    fn formats_bytes() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
    }
}
