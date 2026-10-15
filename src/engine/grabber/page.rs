//! Links of a web page ("deep scan"): `href`/`src` attributes and addresses written as plain
//! text, made absolute against the page address and its `<base href>`. A tolerant scan rather
//! than an HTML parser: pages listing downloads are often malformed, and only links matter.

use std::{collections::HashSet, sync::OnceLock};

use regex::Regex;
use url::Url;

use crate::{
    engine::{magnet, naming},
    util::decode_entities,
};

/// Extensions of files worth downloading: archives, disk images, installers and packages, video,
/// audio, ebooks and documents, torrents. Split archive volumes (`.001`, `.r00`, `.z01`) are
/// recognised separately; `.part1.rar` ends with `.rar`.
const FILE_EXTENSIONS: [&[&str]; 7] = [
    &[
        "7z", "ace", "arj", "bz2", "cab", "gz", "lz", "lzma", "rar", "tar", "tbz2", "tgz", "txz", "xz", "zip", "zipx",
        "zst",
    ],
    &["bin", "cue", "dmg", "img", "iso", "mdf", "mds", "nrg", "qcow2", "vdi", "vhd", "vhdx", "vmdk", "wim"],
    &["apk", "appimage", "appx", "deb", "exe", "flatpak", "jar", "msi", "msix", "pkg", "rpm", "snap", "xapk"],
    &["3gp", "avi", "flv", "m2ts", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "ogv", "ts", "vob", "webm", "wmv"],
    &["aac", "aiff", "alac", "ape", "dsf", "flac", "m4a", "m4b", "mka", "mp3", "ogg", "opus", "wav", "wma"],
    &["azw", "azw3", "cbr", "cbz", "djvu", "epub", "fb2", "mobi", "pdf"],
    &["torrent"],
];

/// Absolute http(s) and magnet links of `html`, deduplicated, in document order. `page` is the
/// address the page was served from (after redirects).
pub fn extract_links(html: &str, page: &Url) -> Vec<String> {
    let base = base_href(html).and_then(|href| page.join(&href).ok()).unwrap_or_else(|| page.clone());
    let mut seen = HashSet::new();
    let mut links = Vec::new();
    for captures in link_pattern().captures_iter(html) {
        let raw = match captures.name("text") {
            Some(text) => text.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}']),
            None => ["double", "single", "bare"].iter().find_map(|g| captures.name(g)).map_or("", |m| m.as_str()),
        };
        let value = decode_entities(raw.trim());
        if let Some(link) = absolute(&base, &value)
            && seen.insert(link.clone())
        {
            links.push(link);
        }
    }
    links
}

/// Whether the name alone says `link` is a file worth downloading: magnets, and paths ending
/// with an archive, disk image, installer, video, audio, ebook or torrent extension.
pub fn is_file_link(link: &str) -> bool {
    if magnet::is_magnet(link) {
        return true;
    }
    let Some(name) = naming::from_url(link) else { return false };
    let Some(extension) = naming::extension(&name) else { return false };
    FILE_EXTENSIONS.iter().any(|kind| kind.contains(&extension.as_str())) || is_volume_extension(&extension)
}

/// `001`, `r00`, `z01`: numbered volumes of split archives.
fn is_volume_extension(extension: &str) -> bool {
    match extension.as_bytes() {
        [first, rest @ ..] if rest.len() == 2 => {
            (first.is_ascii_digit() || matches!(first, b'r' | b'z')) && rest.iter().all(u8::is_ascii_digit)
        }
        _ => false,
    }
}

/// `href`/`src` attribute values (quoted or not), or http(s)/magnet addresses in the text.
fn link_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?i)\b(?:href|src)\s*=\s*(?:"(?P<double>[^"]*)"|'(?P<single>[^']*)'|(?P<bare>[^\s"'<>`]+))|(?P<text>\b(?:https?://|magnet:\?)[^\s"'<>]+)"#,
        )
        .expect("valid regex")
    })
}

/// `href` of the first `<base>` element.
fn base_href(html: &str) -> Option<String> {
    static BASE: OnceLock<Regex> = OnceLock::new();
    let base = BASE.get_or_init(|| {
        Regex::new(r#"(?i)<base\b[^>]*?\bhref\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))"#).expect("valid regex")
    });
    let captures = base.captures(html)?;
    let value = (1..=3).find_map(|group| captures.get(group))?.as_str();
    Some(decode_entities(value.trim()).into_owned())
}

/// The absolute form of an attribute value, if it is an http(s) or magnet link.
fn absolute(base: &Url, value: &str) -> Option<String> {
    if value.is_empty() || value.starts_with('#') {
        return None;
    }
    if magnet::is_magnet(value) {
        return Some(value.to_owned());
    }
    let url = base.join(value).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(html: &str) -> Vec<String> {
        extract_links(html, &Url::parse("https://site.com/posts/42/index.html").unwrap())
    }

    #[test]
    fn resolves_relative_links() {
        let html = r#"<a href="file.zip">a</a> <a href='/top.rar'>b</a> <img src=../img.iso>
            <a href="//cdn.site.com/c.7z">c</a> <a href="https://other.org/d.mkv">d</a>"#;
        assert_eq!(
            links(html),
            [
                "https://site.com/posts/42/file.zip",
                "https://site.com/top.rar",
                "https://site.com/posts/img.iso",
                "https://cdn.site.com/c.7z",
                "https://other.org/d.mkv",
            ]
        );
    }

    #[test]
    fn honours_base_href() {
        let html = r#"<head><BASE target="_blank" HREF="https://mirror.net/dl/"></head><a href="a.zip">"#;
        assert_eq!(links(html), ["https://mirror.net/dl/", "https://mirror.net/dl/a.zip"]);
        let relative = r#"<base href="/files/"><a href="b.zip">"#;
        assert_eq!(links(relative), ["https://site.com/files/", "https://site.com/files/b.zip"]);
    }

    #[test]
    fn decodes_entities_and_deduplicates() {
        let html = r#"<a href="get.php?id=1&amp;name=a%20b.zip">x</a>
            <a href="get.php?id=1&#38;name=a%20b.zip">same</a>
            <a href="magnet:?xt=urn:btih:abc&amp;dn=Show">m</a>
            <p>Mirror: https://mega.nz/file/AbC#k3y-_x, or magnet:?xt=urn:btih:abc&dn=Show.</p>
            <a href="https://site.com/posts/42/get.php?id=1&amp;name=a%20b.zip">again</a>"#;
        assert_eq!(
            links(html),
            [
                "https://site.com/posts/42/get.php?id=1&name=a%20b.zip",
                "magnet:?xt=urn:btih:abc&dn=Show",
                "https://mega.nz/file/AbC#k3y-_x",
            ]
        );
    }

    #[test]
    fn skips_non_links() {
        let html = r##"<a href="#top">t</a> <a href="">e</a> <a href="javascript:void(0)">j</a>
            <a href="mailto:a@b.c">m</a> <img srcset="x.png 2x"> <a href=data:text/plain,hi>d</a>"##;
        assert!(links(html).is_empty());
    }

    #[test]
    fn recognises_file_links() {
        for link in [
            "https://h.com/Movie.2024.part01.rar",
            "https://h.com/backup.7z.001",
            "https://h.com/archive.r00",
            "https://h.com/a/Setup%20v2.EXE?token=x",
            "https://h.com/book.epub",
            "https://h.com/show.torrent",
            "magnet:?xt=urn:btih:abc",
        ] {
            assert!(is_file_link(link), "{link}");
        }
        for link in ["https://h.com/", "https://h.com/page.html", "https://h.com/style.css", "https://h.com/x.r0a"] {
            assert!(!is_file_link(link), "{link}");
        }
    }
}
