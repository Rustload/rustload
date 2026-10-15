//! Checksums: algorithms, checksum files (`.sfv`, `.md5`, `.sha1`, `.sha256`, `.sha512`) and
//! streaming file hashing. Downloads are verified with them before extraction
//! (`engine/verify.rs`).

use std::{fmt, fs::File, io::Read, path::Path, str::FromStr};

use serde::Serialize;
use sha2::Digest;
use tokio_util::sync::CancellationToken;

/// Files are hashed in blocks of this size.
const BLOCK: usize = 1024 * 1024;

/// Supported algorithms, from the weakest to the strongest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HashAlgo {
    Crc32,
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl HashAlgo {
    pub const ALL: [Self; 5] = [Self::Crc32, Self::Md5, Self::Sha1, Self::Sha256, Self::Sha512];

    pub fn name(self) -> &'static str {
        match self {
            Self::Crc32 => "crc32",
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
        }
    }

    /// Length of the hexadecimal digest.
    fn hex_len(self) -> usize {
        match self {
            Self::Crc32 => 8,
            Self::Md5 => 32,
            Self::Sha1 => 40,
            Self::Sha256 => 64,
            Self::Sha512 => 128,
        }
    }

    /// Algorithm of a checksum file, from its extension.
    pub fn of_checksum_file(name: &str) -> Option<Self> {
        let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
        match extension.as_str() {
            "sfv" => Some(Self::Crc32),
            "md5" => Some(Self::Md5),
            "sha1" => Some(Self::Sha1),
            "sha256" => Some(Self::Sha256),
            "sha512" => Some(Self::Sha512),
            _ => None,
        }
    }

    /// Algorithm named in BSD-style lines: `SHA256 (file) = ...`.
    fn from_label(label: &str) -> Option<Self> {
        match label.to_ascii_uppercase().replace('-', "").as_str() {
            "CRC32" => Some(Self::Crc32),
            "MD5" => Some(Self::Md5),
            "SHA1" => Some(Self::Sha1),
            "SHA256" => Some(Self::Sha256),
            "SHA512" => Some(Self::Sha512),
            _ => None,
        }
    }
}

impl fmt::Display for HashAlgo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for HashAlgo {
    type Err = ();

    fn from_str(name: &str) -> Result<Self, ()> {
        Self::ALL.into_iter().find(|algo| algo.name() == name).ok_or(())
    }
}

/// An expected digest: algorithm and lowercase hexadecimal value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHash {
    pub algo: HashAlgo,
    pub hex: String,
}

impl FileHash {
    /// Validates a hexadecimal digest of `algo`.
    pub fn new(algo: HashAlgo, hex: &str) -> Option<Self> {
        let hex = hex.trim();
        (hex.len() == algo.hex_len() && hex.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| Self { algo, hex: hex.to_ascii_lowercase() })
    }
}

/// Stored as `<algo>:<hex>`.
impl fmt::Display for FileHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.algo, self.hex)
    }
}

impl FromStr for FileHash {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, ()> {
        let (algo, hex) = text.split_once(':').ok_or(())?;
        Self::new(algo.parse()?, hex).ok_or(())
    }
}

/// One entry of a checksum file: a `/`-separated path relative to the checksum file, and the
/// digest of that file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksumEntry {
    pub path: String,
    pub hash: FileHash,
}

/// Parses a checksum file named `file_name`. Supported lines:
///
/// - SFV: `name CRC32` (`;` starts a comment);
/// - coreutils: `hash  name` or `hash *name`, with `\` escapes when the line starts with `\`;
/// - BSD: `SHA256 (name) = hash`;
/// - a hash alone, for single-file checksums such as `movie.mkv.md5`.
///
/// Malformed lines are skipped.
pub fn parse_checksum_file(file_name: &str, content: &str) -> Vec<ChecksumEntry> {
    let Some(algo) = HashAlgo::of_checksum_file(file_name) else { return Vec::new() };
    // `movie.mkv.md5` → `movie.mkv`, the target of a lone hash.
    let single_target = file_name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(file_name);
    content.trim_start_matches('\u{feff}').lines().filter_map(|line| parse_line(algo, single_target, line)).collect()
}

fn parse_line(algo: HashAlgo, single_target: &str, line: &str) -> Option<ChecksumEntry> {
    let line = line.trim_end();
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
        return None;
    }
    if let Some(entry) = parse_bsd(trimmed) {
        return Some(entry);
    }
    if algo == HashAlgo::Crc32 {
        let (name, crc) = trimmed.rsplit_once(char::is_whitespace)?;
        return entry(algo, name.trim(), crc, false);
    }
    let (escaped, line) = match trimmed.strip_prefix('\\') {
        Some(rest) => (true, rest),
        None => (false, trimmed),
    };
    match line.split_once(char::is_whitespace) {
        Some((hash, rest)) => {
            // One separator, then ` ` (text mode) or `*` (binary mode).
            let name = rest.strip_prefix([' ', '*']).unwrap_or(rest);
            let name = if escaped { unescape(name) } else { name.to_owned() };
            entry(algo, &name, hash, escaped)
        }
        None => entry(algo, single_target, line, true),
    }
}

/// `SHA256 (name) = hash`.
fn parse_bsd(line: &str) -> Option<ChecksumEntry> {
    let (label, rest) = line.split_once(" (")?;
    let (name, hash) = rest.rsplit_once(") = ")?;
    entry(HashAlgo::from_label(label)?, name, hash, false)
}

/// `literal_path` keeps backslashes: they are only path separators in Windows-made files.
fn entry(algo: HashAlgo, name: &str, hash: &str, literal_path: bool) -> Option<ChecksumEntry> {
    let hash = FileHash::new(algo, hash)?;
    let path = if literal_path { name.to_owned() } else { name.replace('\\', "/") };
    let path = path.trim_start_matches("./").to_owned();
    (!path.is_empty()).then_some(ChecksumEntry { path, hash })
}

/// coreutils escapes: `\\` and `\n` in names of lines starting with a backslash.
fn unescape(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut chars = name.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

enum Hasher {
    Crc32(crc32fast::Hasher),
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
}

impl Hasher {
    fn new(algo: HashAlgo) -> Self {
        match algo {
            HashAlgo::Crc32 => Self::Crc32(crc32fast::Hasher::new()),
            HashAlgo::Md5 => Self::Md5(md5::Md5::new()),
            HashAlgo::Sha1 => Self::Sha1(sha1::Sha1::new()),
            HashAlgo::Sha256 => Self::Sha256(sha2::Sha256::new()),
            HashAlgo::Sha512 => Self::Sha512(sha2::Sha512::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Self::Crc32(hasher) => hasher.update(data),
            Self::Md5(hasher) => hasher.update(data),
            Self::Sha1(hasher) => hasher.update(data),
            Self::Sha256(hasher) => hasher.update(data),
            Self::Sha512(hasher) => hasher.update(data),
        }
    }

    fn finish(self) -> String {
        match self {
            Self::Crc32(hasher) => format!("{:08x}", hasher.finalize()),
            Self::Md5(hasher) => hex::encode(hasher.finalize()),
            Self::Sha1(hasher) => hex::encode(hasher.finalize()),
            Self::Sha256(hasher) => hex::encode(hasher.finalize()),
            Self::Sha512(hasher) => hex::encode(hasher.finalize()),
        }
    }
}

/// Hashes a file block by block, calling `progress` with the number of bytes read after each
/// block. Blocking: run it with `spawn_blocking`. Returns `None` once `cancel` fires.
pub fn hash_file(
    path: &Path,
    algo: HashAlgo,
    cancel: &CancellationToken,
    progress: &mut dyn FnMut(u64),
) -> std::io::Result<Option<String>> {
    let mut file = File::open(path)?;
    let mut hasher = Hasher::new(algo);
    let mut buffer = vec![0u8; BLOCK];
    loop {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        hasher.update(&buffer[..read]);
        progress(read as u64);
    }
    Ok(Some(hasher.finish()))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    const HELLO_MD5: &str = "5d41402abc4b2a76b9719d911017c592";

    fn entries(name: &str, content: &str) -> Vec<(String, String)> {
        parse_checksum_file(name, content).into_iter().map(|e| (e.path, e.hash.to_string())).collect()
    }

    #[test]
    fn parses_sfv() {
        let sfv =
            "; Generated by QuickSFV\r\nMovie.part1.rar 3610A686\r\nSub Dir\\Movie.part2.rar  0d4a1185\r\nbad line\r\n";
        assert_eq!(
            entries("movie.sfv", sfv),
            [
                ("Movie.part1.rar".to_owned(), "crc32:3610a686".to_owned()),
                ("Sub Dir/Movie.part2.rar".to_owned(), "crc32:0d4a1185".to_owned())
            ]
        );
    }

    #[test]
    fn parses_coreutils_lines() {
        let md5 = format!("{HELLO_MD5}  hello.txt\n{HELLO_MD5} *bin ary.dat\n# comment\n{HELLO_MD5} ./sub/x.bin\n");
        assert_eq!(
            entries("files.MD5", &md5),
            [
                ("hello.txt".to_owned(), format!("md5:{HELLO_MD5}")),
                ("bin ary.dat".to_owned(), format!("md5:{HELLO_MD5}")),
                ("sub/x.bin".to_owned(), format!("md5:{HELLO_MD5}"))
            ]
        );
        let escaped = format!("\\{HELLO_MD5}  new\\nline\\\\name\n");
        assert_eq!(entries("a.md5", &escaped), [("new\nline\\name".to_owned(), format!("md5:{HELLO_MD5}"))]);
        // Wrong length for the algorithm of the file.
        assert!(entries("a.sha1", &format!("{HELLO_MD5}  hello.txt")).is_empty());
    }

    #[test]
    fn parses_every_extension() {
        let sha1 = "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d";
        let sha256 = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let sha512 = "9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043";
        assert_eq!(entries("x.sha1", &format!("{sha1}  hello.txt"))[0].1, format!("sha1:{sha1}"));
        assert_eq!(entries("x.sha256", &format!("{sha256} *hello.txt"))[0].1, format!("sha256:{sha256}"));
        assert_eq!(entries("x.sha512", &format!("{sha512}  hello.txt"))[0].1, format!("sha512:{sha512}"));
        assert!(entries("x.txt", &format!("{sha1}  hello.txt")).is_empty());
    }

    #[test]
    fn parses_bsd_lines_and_single_hashes() {
        let bsd = format!(
            "SHA256 (hello.txt) = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824\nMD5 (a (1).txt) = {HELLO_MD5}\n"
        );
        let parsed = entries("CHECKSUMS.sha256", &bsd);
        assert_eq!(parsed[0].0, "hello.txt");
        assert!(parsed[0].1.starts_with("sha256:"));
        assert_eq!(parsed[1], ("a (1).txt".to_owned(), format!("md5:{HELLO_MD5}")));

        assert_eq!(
            entries("movie.mkv.md5", &format!("\u{feff}{HELLO_MD5}\n")),
            [("movie.mkv".to_owned(), format!("md5:{HELLO_MD5}"))]
        );
    }

    #[test]
    fn stores_hashes_as_text() {
        let hash: FileHash = format!("md5:{}", HELLO_MD5.to_uppercase()).parse().unwrap();
        assert_eq!(hash, FileHash { algo: HashAlgo::Md5, hex: HELLO_MD5.to_owned() });
        assert_eq!(hash.to_string(), format!("md5:{HELLO_MD5}"));
        assert!("md5:xyz".parse::<FileHash>().is_err());
        assert!("whirlpool:00".parse::<FileHash>().is_err());
        assert!(HashAlgo::Sha512 > HashAlgo::Md5 && HashAlgo::Md5 > HashAlgo::Crc32);
    }

    #[test]
    fn hashes_files_with_every_algorithm() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::File::create(&path).unwrap().write_all(b"hello").unwrap();
        let cancel = CancellationToken::new();
        let expected = [
            (HashAlgo::Crc32, "3610a686"),
            (HashAlgo::Md5, HELLO_MD5),
            (HashAlgo::Sha1, "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"),
            (HashAlgo::Sha256, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"),
            (
                HashAlgo::Sha512,
                "9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043",
            ),
        ];
        for (algo, digest) in expected {
            let mut read = 0;
            let hash = hash_file(&path, algo, &cancel, &mut |n| read += n).unwrap();
            assert_eq!(hash.as_deref(), Some(digest), "{algo}");
            assert_eq!(read, 5);
        }
        cancel.cancel();
        assert_eq!(hash_file(&path, HashAlgo::Md5, &cancel, &mut |_| {}).unwrap(), None);
        assert!(hash_file(&dir.path().join("missing"), HashAlgo::Md5, &CancellationToken::new(), &mut |_| {}).is_err());
    }
}
