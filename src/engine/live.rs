//! In-memory state of running work: byte counters, speeds and stop signals. Lock-free on the hot
//! path (atomics), sampled once per second for speeds, ETAs and segment maps.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering},
    },
    time::Instant,
};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::transfer::SegmentSet;
use crate::models::DownloadStatus;

/// Speed is averaged over this many one-second samples.
const SPEED_WINDOW: usize = 5;
/// Slices of the segment map, one per percent of the file.
const MAP_SLICES: u64 = 100;

/// Why a running download was asked to stop; decides the status it is left in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// User paused the file or its package → `paused`.
    Paused,
    /// Global pause, schedule, slot reduction or shutdown → back to `queued`.
    Requeue,
    /// File or package deleted → nothing to record.
    Removed,
}

pub struct LiveDownload {
    pub package_id: i64,
    status: AtomicU8,
    pub downloaded: AtomicU64,
    /// Total size, 0 when unknown.
    pub size: AtomicU64,
    pub connections: AtomicU32,
    speed: AtomicU64,
    samples: Mutex<VecDeque<(Instant, u64)>>,
    pub cancel: CancellationToken,
    stop_reason: Mutex<Option<StopReason>>,
    /// File size and segments of a resumable transfer, read when progress is reported.
    segments: Mutex<Option<(u64, Arc<SegmentSet>)>>,
}

impl LiveDownload {
    fn new(package_id: i64) -> Self {
        Self {
            package_id,
            status: AtomicU8::new(0),
            downloaded: AtomicU64::new(0),
            size: AtomicU64::new(0),
            connections: AtomicU32::new(0),
            speed: AtomicU64::new(0),
            samples: Mutex::new(VecDeque::with_capacity(SPEED_WINDOW + 1)),
            cancel: CancellationToken::new(),
            stop_reason: Mutex::new(None),
            segments: Mutex::new(None),
        }
    }

    pub fn set_downloading(&self) {
        self.status.store(1, Ordering::Relaxed);
    }

    pub fn status(&self) -> DownloadStatus {
        if self.status.load(Ordering::Relaxed) == 1 { DownloadStatus::Downloading } else { DownloadStatus::Resolving }
    }

    pub fn speed(&self) -> u64 {
        self.speed.load(Ordering::Relaxed)
    }

    pub fn size(&self) -> Option<u64> {
        Some(self.size.load(Ordering::Relaxed)).filter(|s| *s > 0)
    }

    pub fn eta(&self) -> Option<u64> {
        let speed = self.speed();
        let size = self.size()?;
        (speed > 0).then(|| size.saturating_sub(self.downloaded.load(Ordering::Relaxed)) / speed)
    }

    /// Asks the task to stop. The first reason wins.
    pub fn stop(&self, reason: StopReason) {
        self.stop_reason.lock().expect("poisoned").get_or_insert(reason);
        self.cancel.cancel();
    }

    pub fn stop_reason(&self) -> Option<StopReason> {
        *self.stop_reason.lock().expect("poisoned")
    }

    /// Called by a resumable transfer of known size when it starts, to show its segment map.
    pub(crate) fn track_segments(&self, size: u64, segments: Arc<SegmentSet>) {
        *self.segments.lock().expect("poisoned") = (size > 0).then_some((size, segments));
    }

    /// Segment map (see [`segment_map`]) and connection heads as fractions of the size;
    /// `(None, [])` until a resumable transfer of known size runs.
    pub fn segment_view(&self) -> (Option<String>, Vec<f64>) {
        let Some((size, segments)) = self.segments.lock().expect("poisoned").clone() else {
            return (None, Vec::new());
        };
        let (remaining, heads) = segments.layout();
        let heads = heads.into_iter().map(|pos| (pos as f64 / size as f64 * 10_000.0).round() / 10_000.0).collect();
        (Some(segment_map(size, &remaining)), heads)
    }

    fn sample(&self, now: Instant) {
        let downloaded = self.downloaded.load(Ordering::Relaxed);
        let mut samples = self.samples.lock().expect("poisoned");
        samples.push_back((now, downloaded));
        while samples.len() > SPEED_WINDOW + 1 {
            samples.pop_front();
        }
        let speed = match (samples.front(), samples.back()) {
            (Some((t0, b0)), Some((t1, b1))) if t1 > t0 => {
                (b1.saturating_sub(*b0) as f64 / t1.duration_since(*t0).as_secs_f64()) as u64
            }
            _ => 0,
        };
        self.speed.store(speed, Ordering::Relaxed);
    }
}

/// Snapshot sent in `progress` events and merged into REST responses.
#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub id: i64,
    pub package_id: i64,
    pub status: DownloadStatus,
    pub downloaded: u64,
    pub size: Option<u64>,
    pub speed: u64,
    pub eta: Option<u64>,
    pub connections: u32,
    /// See [`segment_map`]; `None` for single-stream transfers and unknown sizes.
    pub map: Option<String>,
    /// Write position of each active connection, as a fraction of the size.
    pub heads: Vec<f64>,
}

/// Completion of each 1 % slice of a file of `size` bytes, given the byte ranges still missing
/// (half-open, not overlapping): one digit per slice, `floor(fraction × 9)`, so `'9'` means the
/// slice is complete and `'0'`…`'8'` that it is partially downloaded.
pub fn segment_map(size: u64, remaining: &[(u64, u64)]) -> String {
    let bound = |slice: u64| (u128::from(size) * u128::from(slice) / u128::from(MAP_SLICES)) as u64;
    (0..MAP_SLICES)
        .map(|slice| {
            let (start, end) = (bound(slice), bound(slice + 1));
            if start == end {
                // Files under 100 bytes: an empty slice shows the state of its position.
                let missing = remaining.iter().any(|&(from, to)| from <= start && start < to);
                return if missing { '0' } else { '9' };
            }
            let missing: u64 = remaining.iter().map(|&(from, to)| to.min(end).saturating_sub(from.max(start))).sum();
            let length = end - start;
            let done = u128::from(length - missing.min(length));
            char::from(b'0' + (done * 9 / u128::from(length)) as u8)
        })
        .collect()
}

#[derive(Default)]
pub struct LiveRegistry {
    downloads: RwLock<HashMap<i64, Arc<LiveDownload>>>,
    extracting: RwLock<HashMap<i64, Extraction>>,
    verifying: RwLock<HashMap<i64, Extraction>>,
}

/// A running extraction (or checksum verification): progress in thousandths and a way to stop it.
#[derive(Clone)]
pub struct Extraction {
    pub progress: Arc<AtomicU32>,
    pub cancel: CancellationToken,
}

impl LiveRegistry {
    pub fn register(&self, download_id: i64, package_id: i64) -> Arc<LiveDownload> {
        let live = Arc::new(LiveDownload::new(package_id));
        self.downloads.write().expect("poisoned").insert(download_id, live.clone());
        live
    }

    pub fn unregister(&self, download_id: i64) {
        self.downloads.write().expect("poisoned").remove(&download_id);
    }

    pub fn get(&self, download_id: i64) -> Option<Arc<LiveDownload>> {
        self.downloads.read().expect("poisoned").get(&download_id).cloned()
    }

    pub fn is_running(&self, download_id: i64) -> bool {
        self.downloads.read().expect("poisoned").contains_key(&download_id)
    }

    pub fn running_count(&self) -> usize {
        self.downloads.read().expect("poisoned").len()
    }

    pub fn all(&self) -> Vec<(i64, Arc<LiveDownload>)> {
        self.downloads.read().expect("poisoned").iter().map(|(id, live)| (*id, live.clone())).collect()
    }

    /// Stops every running download of a package.
    pub fn stop_package(&self, package_id: i64, reason: StopReason) {
        for (_, live) in self.all() {
            if live.package_id == package_id {
                live.stop(reason);
            }
        }
    }

    pub fn stop_all(&self, reason: StopReason) {
        for (_, live) in self.all() {
            live.stop(reason);
        }
    }

    /// Updates speeds; returns the total speed.
    pub fn sample(&self) -> u64 {
        let now = Instant::now();
        self.all()
            .iter()
            .map(|(_, live)| {
                live.sample(now);
                live.speed()
            })
            .sum()
    }

    pub fn progress(&self) -> Vec<DownloadProgress> {
        let mut items: Vec<DownloadProgress> = self
            .all()
            .into_iter()
            .map(|(id, live)| {
                let (map, heads) = live.segment_view();
                DownloadProgress {
                    id,
                    package_id: live.package_id,
                    status: live.status(),
                    downloaded: live.downloaded.load(Ordering::Relaxed),
                    size: live.size(),
                    speed: live.speed(),
                    eta: live.eta(),
                    connections: live.connections.load(Ordering::Relaxed),
                    map,
                    heads,
                }
            })
            .collect();
        items.sort_by_key(|p| p.id);
        items
    }

    /// Registers an extraction; `parent` cancels it too (shutdown).
    pub fn start_extraction(&self, package_id: i64, parent: &CancellationToken) -> Extraction {
        let extraction = Extraction { progress: Arc::new(AtomicU32::new(0)), cancel: parent.child_token() };
        self.extracting.write().expect("poisoned").insert(package_id, extraction.clone());
        extraction
    }

    pub fn cancel_extraction(&self, package_id: i64) {
        if let Some(extraction) = self.extracting.read().expect("poisoned").get(&package_id) {
            extraction.cancel.cancel();
        }
    }

    pub fn is_extracting(&self, package_id: i64) -> bool {
        self.extracting.read().expect("poisoned").contains_key(&package_id)
    }

    pub fn finish_extraction(&self, package_id: i64) {
        self.extracting.write().expect("poisoned").remove(&package_id);
    }

    pub fn extraction_progress(&self, package_id: i64) -> Option<f64> {
        self.extracting
            .read()
            .expect("poisoned")
            .get(&package_id)
            .map(|e| e.progress.load(Ordering::Relaxed) as f64 / 1000.0)
    }

    pub fn extracting_ids(&self) -> Vec<i64> {
        self.extracting.read().expect("poisoned").keys().copied().collect()
    }

    /// Registers a checksum verification; `parent` cancels it too (shutdown).
    pub fn start_verification(&self, package_id: i64, parent: &CancellationToken) -> Extraction {
        let verification = Extraction { progress: Arc::new(AtomicU32::new(0)), cancel: parent.child_token() };
        self.verifying.write().expect("poisoned").insert(package_id, verification.clone());
        verification
    }

    pub fn cancel_verification(&self, package_id: i64) {
        if let Some(verification) = self.verifying.read().expect("poisoned").get(&package_id) {
            verification.cancel.cancel();
        }
    }

    pub fn is_verifying(&self, package_id: i64) -> bool {
        self.verifying.read().expect("poisoned").contains_key(&package_id)
    }

    pub fn finish_verification(&self, package_id: i64) {
        self.verifying.write().expect("poisoned").remove(&package_id);
    }

    pub fn verification_progress(&self, package_id: i64) -> Option<f64> {
        self.verifying
            .read()
            .expect("poisoned")
            .get(&package_id)
            .map(|v| v.progress.load(Ordering::Relaxed) as f64 / 1000.0)
    }

    pub fn verifying_ids(&self) -> Vec<i64> {
        self.verifying.read().expect("poisoned").keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_complete_and_missing_slices() {
        assert_eq!(segment_map(1000, &[]), "9".repeat(100));
        assert_eq!(segment_map(1000, &[(0, 1000)]), "0".repeat(100));
        assert_eq!(segment_map(1000, &[(0, 500)]), "0".repeat(50) + &"9".repeat(50));
    }

    #[test]
    fn grades_partial_slices() {
        // Slices are 10 bytes long here.
        assert_eq!(&segment_map(1000, &[(5, 10)])[..2], "49");
        assert_eq!(&segment_map(1000, &[(9, 10)])[..1], "8");
        assert_eq!(&segment_map(1000, &[(0, 9)])[..1], "0");
        assert_eq!(&segment_map(1000, &[(15, 35), (990, 1000)])[..5], "94049");
        assert!(segment_map(1000, &[(15, 35), (990, 1000)]).ends_with("90"));
    }

    #[test]
    fn handles_tiny_and_huge_files() {
        assert_eq!(segment_map(10, &[(5, 10)]), "9".repeat(50) + &"0".repeat(50));
        assert_eq!(segment_map(u64::MAX, &[(u64::MAX / 2, u64::MAX)]), "9".repeat(50) + &"0".repeat(50));
    }

    #[test]
    fn shows_no_map_before_a_transfer() {
        assert_eq!(LiveDownload::new(1).segment_view(), (None, Vec::new()));
    }
}
