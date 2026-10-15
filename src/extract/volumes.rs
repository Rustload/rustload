//! Recognises archives and groups multi-volume sets by file name.
//!
//! Supported layouts: `x.part1.rar…`, `x.rar` + `x.r00…` (+ `x.s00…`), `x.7z` / `x.7z.001…`,
//! `x.zip` / `x.zip.001…` / `x.z01…` + `x.zip`, tarballs, and generic `x.ext.001…` splits.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArchiveKind {
    Rar,
    SevenZip,
    Zip,
    Tar,
    /// Plain file cut in pieces (`movie.mkv.001`, HJSplit).
    Split,
}

impl ArchiveKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rar => "RAR",
            Self::SevenZip => "7z",
            Self::Zip => "ZIP",
            Self::Tar => "tar",
            Self::Split => "split",
        }
    }
}

/// How volumes are numbered, used to detect gaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Numbering {
    /// `part1`, `001`: must be contiguous from 1 (or 0).
    Sequential,
    /// `x.rar` is volume 0, `x.r00` volume 1, ..., `x.s00` volume 101.
    OldRar,
    /// `x.z01`… then `x.zip`, which is opened first.
    SpannedZip,
    Single,
}

#[derive(Debug, Clone)]
pub struct ArchiveSet {
    pub kind: ArchiveKind,
    /// The file to open.
    pub first: PathBuf,
    /// Every volume of the set (deleted after a successful extraction).
    pub volumes: Vec<PathBuf>,
    numbering: Numbering,
    numbers: Vec<u32>,
}

impl ArchiveSet {
    /// Name of the first missing volume, if the set has a gap.
    pub fn missing_volume(&self) -> Option<String> {
        let mut numbers = self.numbers.clone();
        numbers.sort_unstable();
        if numbers.is_empty() {
            // A lone `.zip` (its `.zNN` companions would have numbers).
            return None;
        }
        let start = match self.numbering {
            Numbering::Single => return None,
            Numbering::Sequential => numbers.first().copied().unwrap_or(1).min(1),
            Numbering::OldRar => 0,
            Numbering::SpannedZip => 1,
        };
        let max = numbers.last().copied().unwrap_or(start);
        (start..=max)
            .find(|n| numbers.binary_search(n).is_err())
            .map(|n| format!("volume {n} of {}", self.first.file_name().unwrap_or_default().to_string_lossy()))
    }
}

struct Classified {
    kind: ArchiveKind,
    numbering: Numbering,
    stem: String,
    number: u32,
    /// Whether this volume is the one to open for its set.
    opener: bool,
}

fn rules() -> &'static [(Regex, ArchiveKind, Numbering)] {
    static RULES: OnceLock<Vec<(Regex, ArchiveKind, Numbering)>> = OnceLock::new();
    RULES.get_or_init(|| {
        let rule = |pattern: &str, kind, numbering| (Regex::new(pattern).expect("valid regex"), kind, numbering);
        vec![
            rule(r"^(?P<stem>.+)\.part(?P<n>\d+)\.rar$", ArchiveKind::Rar, Numbering::Sequential),
            rule(r"^(?P<stem>.+)\.rar$", ArchiveKind::Rar, Numbering::OldRar),
            rule(r"^(?P<stem>.+)\.r(?P<n>\d{2,3})$", ArchiveKind::Rar, Numbering::OldRar),
            rule(r"^(?P<stem>.+)\.s(?P<s>\d{2})$", ArchiveKind::Rar, Numbering::OldRar),
            rule(r"^(?P<stem>.+)\.7z\.(?P<n>\d{3})$", ArchiveKind::SevenZip, Numbering::Sequential),
            rule(r"^(?P<stem>.+)\.7z$", ArchiveKind::SevenZip, Numbering::Single),
            rule(r"^(?P<stem>.+)\.zip\.(?P<n>\d{3})$", ArchiveKind::Zip, Numbering::Sequential),
            rule(r"^(?P<stem>.+)\.z(?P<n>\d{2})$", ArchiveKind::Zip, Numbering::SpannedZip),
            rule(r"^(?P<stem>.+)\.zip$", ArchiveKind::Zip, Numbering::SpannedZip),
            rule(
                r"^(?P<stem>.+)\.(?:tar|tar\.gz|tgz|tar\.bz2|tbz2?|tar\.xz|txz|tar\.zst|tzst)$",
                ArchiveKind::Tar,
                Numbering::Single,
            ),
            rule(r"^(?P<stem>.+)\.(?P<n>\d{3})$", ArchiveKind::Split, Numbering::Sequential),
        ]
    })
}

fn classify(name: &str) -> Option<Classified> {
    let lower = name.to_ascii_lowercase();
    for (regex, kind, numbering) in rules() {
        let Some(captures) = regex.captures(&lower) else { continue };
        let stem = captures["stem"].to_owned();
        let (number, opener) = match (kind, numbering) {
            (_, Numbering::Single) => (0, true),
            (_, Numbering::Sequential) => {
                let n: u32 = captures["n"].parse().ok()?;
                (n, n <= 1)
            }
            (ArchiveKind::Rar, Numbering::OldRar) => match (captures.name("n"), captures.name("s")) {
                (Some(n), _) => (n.as_str().parse::<u32>().ok()? + 1, false),
                (_, Some(s)) => (s.as_str().parse::<u32>().ok()? + 101, false),
                _ => (0, true),
            },
            (_, Numbering::SpannedZip) => match captures.name("n") {
                Some(n) => (n.as_str().parse().ok()?, false),
                // `.zip` closes a spanned set; numbered after every `.zNN`.
                None => (u32::MAX, true),
            },
            _ => (0, true),
        };
        return Some(Classified { kind: *kind, numbering: *numbering, stem, number, opener });
    }
    None
}

/// Folder, stem and layout identifying the volumes of one set.
type SetKey = (PathBuf, String, ArchiveKind, Numbering);

/// Groups `files` into archive sets. Sets whose opening volume is not among the files are
/// ignored (nothing to open).
pub fn find_archives(files: &[PathBuf]) -> Vec<ArchiveSet> {
    let mut groups: BTreeMap<SetKey, Vec<(PathBuf, Classified)>> = BTreeMap::new();
    for path in files {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        let Some(classified) = classify(&name) else { continue };
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        groups
            .entry((dir, classified.stem.clone(), classified.kind, classified.numbering))
            .or_default()
            .push((path.clone(), classified));
    }

    groups
        .into_values()
        .filter_map(|mut members| {
            members.sort_by_key(|(_, c)| c.number);
            let first = members.iter().filter(|(_, c)| c.opener).min_by_key(|(_, c)| c.number)?.0.clone();
            let (kind, numbering) = (members[0].1.kind, members[0].1.numbering);
            let numbers = members.iter().map(|(_, c)| c.number).filter(|n| *n != u32::MAX).collect();
            Some(ArchiveSet { kind, first, volumes: members.into_iter().map(|(p, _)| p).collect(), numbering, numbers })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sets(names: &[&str]) -> Vec<ArchiveSet> {
        let files: Vec<PathBuf> = names.iter().map(|n| PathBuf::from("/d").join(n)).collect();
        find_archives(&files)
    }

    fn first_name(set: &ArchiveSet) -> String {
        set.first.file_name().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn groups_part_style_rar() {
        let found = sets(&["Movie.part2.rar", "Movie.part1.rar", "Movie.part10.rar", "readme.txt"]);
        assert_eq!(found.len(), 1);
        assert_eq!(first_name(&found[0]), "Movie.part1.rar");
        assert_eq!(found[0].volumes.len(), 3);
        assert_eq!(found[0].missing_volume().as_deref(), Some("volume 3 of Movie.part1.rar"));
    }

    #[test]
    fn groups_old_style_rar() {
        let found = sets(&["a.r01", "a.rar", "a.r00"]);
        assert_eq!(found.len(), 1);
        assert_eq!(first_name(&found[0]), "a.rar");
        assert_eq!(found[0].volumes.len(), 3);
        assert!(found[0].missing_volume().is_none());
    }

    #[test]
    fn detects_missing_first_rar_volume() {
        let found = sets(&["a.r00"]);
        assert!(found.is_empty(), "no opener → nothing to extract");
    }

    #[test]
    fn groups_split_formats() {
        let found = sets(&["b.7z.002", "b.7z.001", "c.mkv.001", "c.mkv.002", "d.z01", "d.zip", "e.tar.gz", "f.zip"]);
        let mut summary: Vec<(ArchiveKind, String, usize)> =
            found.iter().map(|s| (s.kind, first_name(s), s.volumes.len())).collect();
        summary.sort();
        assert_eq!(
            summary,
            vec![
                (ArchiveKind::SevenZip, "b.7z.001".into(), 2),
                (ArchiveKind::Zip, "d.zip".into(), 2),
                (ArchiveKind::Zip, "f.zip".into(), 1),
                (ArchiveKind::Tar, "e.tar.gz".into(), 1),
                (ArchiveKind::Split, "c.mkv.001".into(), 2),
            ]
        );
        assert!(found.iter().all(|s| s.missing_volume().is_none()));
    }

    #[test]
    fn ignores_regular_files() {
        assert!(sets(&["movie.mkv", "song.mp3", "notes.txt"]).is_empty());
    }
}
