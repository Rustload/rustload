//! File names coming from the network are untrusted: they are sanitised before touching the disk
//! (no path separators, no traversal, no characters SMB/Windows shares reject).

use std::path::{Path, PathBuf};

use std::sync::OnceLock;

use percent_encoding::percent_decode_str;
use regex::Regex;

/// Byte budget for a file name (ext4/NTFS allow 255 bytes).
const MAX_NAME_BYTES: usize = 240;
const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Makes `name` safe as a single path component. Never returns an empty string, `.` or `..`.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).trim_start_matches(' ');
    let mut cleaned = if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        "download".to_owned()
    } else {
        cleaned.to_owned()
    };
    let stem = cleaned.split('.').next().unwrap_or_default().to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem.as_str()) {
        cleaned.insert(0, '_');
    }
    truncate_keeping_extension(&cleaned, MAX_NAME_BYTES)
}

/// Sanitises every component of a relative `/`-separated path (torrent folder structure).
pub fn sanitize_relative_dir(path: &str) -> PathBuf {
    path.split(['/', '\\']).filter(|part| !part.is_empty() && *part != "." && *part != "..").map(sanitize).collect()
}

fn truncate_keeping_extension(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_owned();
    }
    let (stem, ext) = split_extension(name);
    let ext_len = ext.map(|e| e.len() + 1).unwrap_or(0).min(max / 2);
    let mut cut = max - ext_len;
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    match ext {
        Some(ext) if ext.len() < max / 2 => format!("{}.{ext}", &stem[..cut]),
        _ => {
            let mut cut = max;
            while !name.is_char_boundary(cut) {
                cut -= 1;
            }
            name[..cut].to_owned()
        }
    }
}

/// `("movie", Some("mkv"))`; dotfiles have no extension.
pub fn split_extension(name: &str) -> (&str, Option<&str>) {
    match name.rfind('.') {
        Some(0) | None => (name, None),
        Some(index) => (&name[..index], Some(&name[index + 1..])),
    }
}

/// Lower-case extension used for category matching (`movie.part1.rar` → `rar`).
pub fn extension(name: &str) -> Option<String> {
    split_extension(name).1.map(str::to_ascii_lowercase)
}

/// File name from a `Content-Disposition` header, preferring the RFC 5987 `filename*` form.
pub fn from_content_disposition(header: &[u8]) -> Option<String> {
    let value = String::from_utf8_lossy(header);
    let mut plain = None;
    for part in split_params(&value) {
        let Some((key, raw)) = part.split_once('=') else { continue };
        let key = key.trim().to_ascii_lowercase();
        let raw = raw.trim();
        if key == "filename*" {
            // charset'language'percent-encoded
            let mut pieces = raw.trim_matches('"').splitn(3, '\'');
            let charset = pieces.next().unwrap_or_default().to_ascii_lowercase();
            let _language = pieces.next();
            if let Some(encoded) = pieces.next() {
                let bytes: Vec<u8> = percent_decode_str(encoded).collect();
                let decoded = if charset == "iso-8859-1" {
                    bytes.iter().map(|&b| b as char).collect()
                } else {
                    String::from_utf8_lossy(&bytes).into_owned()
                };
                if !decoded.trim().is_empty() {
                    return Some(decoded);
                }
            }
        } else if key == "filename" {
            let unquoted = if raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2 {
                raw[1..raw.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
            } else {
                raw.to_owned()
            };
            // Some servers percent-encode the plain form too.
            let decoded = if unquoted.contains('%') {
                percent_decode_str(&unquoted).decode_utf8_lossy().into_owned()
            } else {
                unquoted
            };
            if !decoded.trim().is_empty() {
                plain = Some(decoded);
            }
        }
    }
    plain
}

/// Splits on `;` outside of double quotes.
fn split_params(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut in_quotes = false;
    let mut start = 0;
    let mut escaped = false;
    for (index, c) in value.char_indices() {
        match c {
            '\\' if in_quotes && !escaped => {
                escaped = true;
                continue;
            }
            '"' if !escaped => in_quotes = !in_quotes,
            ';' if !in_quotes => {
                parts.push(&value[start..index]);
                start = index + 1;
            }
            _ => {}
        }
        escaped = false;
    }
    parts.push(&value[start..]);
    parts
}

/// Last path segment of a URL, percent-decoded.
pub fn from_url(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    let segment = url.path_segments()?.rev().find(|s| !s.is_empty())?;
    let decoded = percent_decode_str(segment).decode_utf8_lossy().into_owned();
    (!decoded.trim().is_empty()).then_some(decoded)
}

/// `dir/name`, or `dir/name (1).ext`, `dir/name (2).ext`... if taken. The number goes before
/// archive volume suffixes (`Movie (1).part1.rar`) so that renamed volumes still form a set.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, suffix) = split_for_numbering(name);
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){suffix}")))
        .find(|path| !path.exists())
        .expect("an unused name always exists")
}

/// `("Movie", ".part1.rar")`, `("data", ".tar.gz")`, `("film", ".mkv")`, `("README", "")`.
fn split_for_numbering(name: &str) -> (&str, &str) {
    static VOLUME: OnceLock<Regex> = OnceLock::new();
    let volume = VOLUME.get_or_init(|| {
        Regex::new(
            r"(?i)(\.part\d+\.rar|\.r\d{2,3}|\.s\d{2}|\.(?:7z|zip)\.\d{3}|\.z\d{2}|\.\d{3}|\.tar\.(?:gz|bz2|xz|zst))$",
        )
        .expect("valid regex")
    });
    match volume.find(name).filter(|m| m.start() > 0) {
        Some(m) => (&name[..m.start()], &name[m.start()..]),
        None => match split_extension(name) {
            (stem, Some(_)) => (stem, &name[stem.len()..]),
            (stem, None) => (stem, ""),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_dangerous_names() {
        assert_eq!(sanitize("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(sanitize(".."), "download");
        assert_eq!(sanitize("  movie: part 1?.mkv "), "movie_ part 1_.mkv");
        assert_eq!(sanitize("CON.txt"), "_CON.txt");
        assert_eq!(sanitize("name.   "), "name");
        assert_eq!(sanitize(""), "download");
    }

    #[test]
    fn truncates_long_names_keeping_extension() {
        let long = format!("{}.mkv", "é".repeat(200));
        let result = sanitize(&long);
        assert!(result.len() <= MAX_NAME_BYTES);
        assert!(result.ends_with(".mkv"));
    }

    #[test]
    fn sanitizes_relative_dirs() {
        assert_eq!(sanitize_relative_dir("Show/../Season 1/"), PathBuf::from("Show").join("Season 1"));
    }

    #[test]
    fn parses_content_disposition() {
        let cd = |v: &str| from_content_disposition(v.as_bytes());
        assert_eq!(cd(r#"attachment; filename="a b.zip""#).as_deref(), Some("a b.zip"));
        assert_eq!(cd("attachment; filename=plain.rar").as_deref(), Some("plain.rar"));
        assert_eq!(
            cd(r#"attachment; filename="fallback.txt"; filename*=UTF-8''%C3%A9t%C3%A9.mkv"#).as_deref(),
            Some("été.mkv")
        );
        assert_eq!(cd(r#"attachment; filename="semi;colon.txt""#).as_deref(), Some("semi;colon.txt"));
        assert_eq!(cd("inline").as_deref(), None);
    }

    #[test]
    fn names_from_urls() {
        assert_eq!(from_url("https://h.com/files/My%20Movie.mkv?x=1").as_deref(), Some("My Movie.mkv"));
        assert_eq!(from_url("https://h.com/dir/").as_deref(), Some("dir"));
        assert_eq!(from_url("https://h.com/").as_deref(), None);
    }

    #[test]
    fn finds_unique_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"").unwrap();
        assert_eq!(unique_path(dir.path(), "a.txt"), dir.path().join("a (1).txt"));
        assert_eq!(unique_path(dir.path(), "b.txt"), dir.path().join("b.txt"));
        std::fs::write(dir.path().join("Movie.part1.rar"), b"").unwrap();
        assert_eq!(unique_path(dir.path(), "Movie.part1.rar"), dir.path().join("Movie (1).part1.rar"));
        std::fs::write(dir.path().join("data.tar.gz"), b"").unwrap();
        assert_eq!(unique_path(dir.path(), "data.tar.gz"), dir.path().join("data (1).tar.gz"));
    }
}
