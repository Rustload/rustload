//! Download statistics.
//!
//! Completed files are counted as they finish, per local day and hour, host and handler (`via`),
//! in tables that do not depend on the history (clearing it keeps the numbers). Files and packages
//! count once (`counted` columns), even when downloaded or extracted again. The peak speed of
//! the day is sampled by the engine's one-second ticker and kept in memory: it is written when it
//! rises, at most once a minute, and on shutdown.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use chrono::{Days, Local, NaiveDate, Timelike};
use serde::Serialize;

use crate::{
    db::{
        self, Db,
        stats::{AllTimeRow, CompletedFile, DayRow, TransferRow},
    },
    engine::{DiskInfo, Engine, live::DownloadProgress},
};

/// Hosts listed by name in a report; the others are summed up as [`OTHER_HOSTS`].
const TOP_HOSTS: usize = 8;
pub const OTHER_HOSTS: &str = "other";
const PEAK_FLUSH_INTERVAL: Duration = Duration::from_secs(60);

/// Records statistics from the engine. Failures are logged and never interrupt downloads.
pub struct StatsRecorder {
    db: Db,
    peak: Mutex<Peak>,
}

impl StatsRecorder {
    pub fn new(db: Db) -> Self {
        Self { db, peak: Mutex::new(Peak::new(Local::now().date_naive())) }
    }

    /// Counts download `id`, completed with `size` bytes, unless it was counted before: a file
    /// downloaded again (checksum mismatch, retry) counts once. `transferred` bytes were
    /// downloaded in `elapsed` by the last run (less than `size` when it resumed an earlier one).
    pub async fn file_completed(
        &self,
        id: i64,
        host: Option<&str>,
        via: Option<&str>,
        size: u64,
        transferred: u64,
        elapsed: Duration,
    ) {
        match db::downloads::mark_counted(&self.db, id).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                tracing::warn!("cannot record statistics: {error:#}");
                return;
            }
        }
        let now = Local::now();
        let day = day_key(now.date_naive());
        let file = CompletedFile {
            day: &day,
            hour: now.hour(),
            host: host.unwrap_or_default(),
            via: via.unwrap_or("direct"),
            bytes: i64::try_from(size).unwrap_or(i64::MAX),
            secs: transfer_secs(size, transferred, elapsed),
            at: now.timestamp(),
        };
        if let Err(error) = db::stats::add_transfer(&self.db, &file).await {
            tracing::warn!("cannot record statistics: {error:#}");
        }
    }

    /// Counts a file that failed for good.
    pub async fn file_failed(&self) {
        if let Err(error) = db::stats::add_failure(&self.db, &today_key()).await {
            tracing::warn!("cannot record statistics: {error:#}");
        }
    }

    /// Counts package `id`, which completed successfully, unless it was counted before (a package
    /// extracted again from the history counts once).
    pub async fn package_finished(&self, id: i64) {
        let counted = async {
            if db::packages::mark_counted(&self.db, id).await? {
                db::stats::add_package(&self.db, &today_key()).await?;
            }
            anyhow::Ok(())
        };
        if let Err(error) = counted.await {
            tracing::warn!("cannot record statistics: {error:#}");
        }
    }

    /// Takes the total speed of the current second into account for the peak of the day.
    pub async fn sample_speed(&self, speed: u64) {
        let writes = self.peak.lock().expect("poisoned").sample(Local::now().date_naive(), speed, Instant::now());
        for (day, speed) in writes {
            self.write_peak(day, speed).await;
        }
    }

    /// Writes a peak that is still only in memory (shutdown).
    pub async fn flush(&self) {
        let pending = self.peak.lock().expect("poisoned").take_pending();
        if let Some((day, speed)) = pending {
            self.write_peak(day, speed).await;
        }
    }

    async fn write_peak(&self, day: NaiveDate, speed: u64) {
        let speed = i64::try_from(speed).unwrap_or(i64::MAX);
        if let Err(error) = db::stats::raise_peak(&self.db, &day_key(day), speed).await {
            tracing::warn!("cannot record the peak speed: {error:#}");
        }
    }
}

/// Highest total speed of the current day, and the part of it already written.
struct Peak {
    day: NaiveDate,
    speed: u64,
    stored: u64,
    flushed_at: Option<Instant>,
}

impl Peak {
    fn new(day: NaiveDate) -> Self {
        Self { day, speed: 0, stored: 0, flushed_at: None }
    }

    /// Takes a sample and returns the peaks to write now: the previous day's when the day changed,
    /// and today's when it rose and was not written during the last minute.
    fn sample(&mut self, today: NaiveDate, speed: u64, now: Instant) -> Vec<(NaiveDate, u64)> {
        let mut writes = Vec::new();
        if self.day != today {
            writes.extend(self.take_pending());
            *self = Self::new(today);
        }
        self.speed = self.speed.max(speed);
        if self.flushed_at.is_none_or(|at| now.duration_since(at) >= PEAK_FLUSH_INTERVAL)
            && let Some(pending) = self.take_pending()
        {
            writes.push(pending);
            self.flushed_at = Some(now);
        }
        writes
    }

    fn take_pending(&mut self) -> Option<(NaiveDate, u64)> {
        (self.speed > self.stored).then(|| {
            self.stored = self.speed;
            (self.day, self.speed)
        })
    }
}

/// Transfer time to record for a file. A resumed file was only timed for its last part: the time
/// is scaled to the whole file so that bytes ÷ seconds stays the measured speed.
fn transfer_secs(size: u64, transferred: u64, elapsed: Duration) -> f64 {
    let secs = elapsed.as_secs_f64();
    if transferred == 0 || transferred >= size { secs } else { secs * size as f64 / transferred as f64 }
}

fn day_key(day: NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

fn today_key() -> String {
    day_key(Local::now().date_naive())
}

/// `GET /api/stats` (see `docs/API.md`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub days: u32,
    pub totals: Totals,
    pub all_time: AllTime,
    /// Every day of the range, oldest first.
    pub daily: Vec<DailyTotal>,
    pub hosts: Vec<HostTotal>,
    pub providers: Vec<ProviderTotal>,
    /// Bytes completed per local hour of the day over the range.
    pub hours: [u64; 24],
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Totals {
    pub bytes: u64,
    pub files: u64,
    pub packages: u64,
    pub failed: u64,
    pub avg_speed: Option<u64>,
    pub peak_speed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AllTime {
    pub bytes: u64,
    pub files: u64,
    pub since: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DailyTotal {
    pub day: String,
    pub bytes: u64,
    pub files: u64,
    pub failed: u64,
    pub peak_speed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HostTotal {
    pub host: String,
    pub bytes: u64,
    pub files: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderTotal {
    pub via: String,
    pub bytes: u64,
    pub files: u64,
}

/// Statistics of the `days` days ending with `today` (local dates).
pub async fn report(db: &Db, days: u32, today: NaiveDate) -> anyhow::Result<Report> {
    let first = day_key(first_day(days, today));
    let last = day_key(today);
    let transfers = db::stats::transfers_between(db, &first, &last).await?;
    let day_rows = db::stats::days_between(db, &first, &last).await?;
    let all_time = db::stats::all_time(db).await?;
    Ok(build_report(days, today, &transfers, &day_rows, &all_time))
}

fn first_day(days: u32, today: NaiveDate) -> NaiveDate {
    today - Days::new(u64::from(days.max(1) - 1))
}

/// Aggregates database rows into a report; days without activity are filled with zeros.
pub fn build_report(
    days: u32,
    today: NaiveDate,
    transfers: &[TransferRow],
    day_rows: &[DayRow],
    all_time: &AllTimeRow,
) -> Report {
    let days = days.max(1);
    let first = first_day(days, today);
    let index = |day: &str| {
        let offset = (NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()? - first).num_days();
        usize::try_from(offset).ok().filter(|i| *i < days as usize)
    };
    let mut daily: Vec<DailyTotal> = (0..days)
        .map(|i| DailyTotal {
            day: day_key(first + Days::new(u64::from(i))),
            bytes: 0,
            files: 0,
            failed: 0,
            peak_speed: None,
        })
        .collect();
    let mut totals = Totals::default();
    let mut hours = [0u64; 24];
    let mut hosts: HashMap<&str, (u64, u64)> = HashMap::new();
    let mut providers: HashMap<&str, (u64, u64)> = HashMap::new();
    let mut secs = 0.0;

    for row in transfers {
        let Some(i) = index(&row.day) else { continue };
        let (bytes, files) = (unsigned(row.bytes), unsigned(row.files));
        daily[i].bytes += bytes;
        daily[i].files += files;
        totals.bytes += bytes;
        totals.files += files;
        secs += row.secs.max(0.0);
        if let Some(slot) = usize::try_from(row.hour).ok().and_then(|hour| hours.get_mut(hour)) {
            *slot += bytes;
        }
        for (map, key) in [(&mut hosts, row.host.as_str()), (&mut providers, row.via.as_str())] {
            let entry = map.entry(key).or_default();
            entry.0 += bytes;
            entry.1 += files;
        }
    }
    totals.avg_speed = (secs > 0.0).then(|| (totals.bytes as f64 / secs).round() as u64);

    for row in day_rows {
        let Some(i) = index(&row.day) else { continue };
        let peak = row.peak_speed.map(unsigned);
        daily[i].failed += unsigned(row.failed);
        daily[i].peak_speed = daily[i].peak_speed.max(peak);
        totals.packages += unsigned(row.packages);
        totals.failed += unsigned(row.failed);
        totals.peak_speed = totals.peak_speed.max(peak);
    }

    let mut hosts = by_bytes(hosts);
    if hosts.len() > TOP_HOSTS {
        let other = hosts.split_off(TOP_HOSTS).into_iter().fold((0, 0), |acc, (_, (b, f))| (acc.0 + b, acc.1 + f));
        hosts.push((OTHER_HOSTS, other));
    }

    Report {
        days,
        totals,
        all_time: AllTime { bytes: unsigned(all_time.bytes), files: unsigned(all_time.files), since: all_time.since },
        daily,
        hosts: hosts
            .into_iter()
            .map(|(host, (bytes, files))| HostTotal { host: host.to_owned(), bytes, files })
            .collect(),
        providers: by_bytes(providers)
            .into_iter()
            .map(|(via, (bytes, files))| ProviderTotal { via: via.to_owned(), bytes, files })
            .collect(),
        hours,
    }
}

/// Largest first; ties by name so that the order is stable.
fn by_bytes(map: HashMap<&str, (u64, u64)>) -> Vec<(&str, (u64, u64))> {
    let mut entries: Vec<_> = map.into_iter().collect();
    entries.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
    entries
}

fn unsigned(value: i64) -> u64 {
    value.max(0) as u64
}

/// What the queue is doing right now; shared by `/metrics` and the Telegram bot.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Total speed, bytes/s.
    pub speed: u64,
    /// Files transferring (or being resolved).
    pub active: usize,
    /// Files waiting for a slot.
    pub queued: u64,
    /// Bytes left to download in the active and queued files.
    pub remaining: u64,
    /// Global pause.
    pub paused: bool,
    /// `false` outside the download window of the schedule.
    pub schedule_open: bool,
    pub disk: Option<DiskInfo>,
}

impl Snapshot {
    pub async fn take(engine: &Engine) -> anyhow::Result<Self> {
        let active = engine.live.progress();
        let queue = db::downloads::queue_remaining_bytes(&engine.db).await?;
        Ok(Self::of(engine, &active, queue))
    }

    /// From the progress of the active files and `(bytes left, count)` of the queued ones.
    pub fn of(engine: &Engine, active: &[DownloadProgress], (queued_remaining, queued): (i64, i64)) -> Self {
        let active_remaining: u64 = active.iter().map(|d| d.size.unwrap_or(0).saturating_sub(d.downloaded)).sum();
        Self {
            speed: active.iter().map(|d| d.speed).sum(),
            active: active.len(),
            queued: unsigned(queued),
            remaining: unsigned(queued_remaining) + active_remaining,
            paused: engine.is_paused(),
            schedule_open: engine.schedule_open(),
            disk: engine.disk_info(),
        }
    }

    /// Seconds until the queue is done at the current speed.
    pub fn eta(&self) -> Option<u64> {
        (self.speed > 0 && self.remaining > 0).then(|| self.remaining / self.speed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    fn transfer(day: &str, hour: i64, host: &str, via: &str, bytes: i64, files: i64, secs: f64) -> TransferRow {
        TransferRow { day: day.into(), hour, host: host.into(), via: via.into(), bytes, files, secs }
    }

    fn day_row(day: &str, packages: i64, failed: i64, peak_speed: Option<i64>) -> DayRow {
        DayRow { day: day.into(), packages, failed, peak_speed }
    }

    #[test]
    fn fills_every_day_of_the_range() {
        let transfers = [
            transfer("2026-10-14", 9, "a.com", "direct", 1000, 2, 4.0),
            transfer("2026-10-14", 21, "b.com", "alldebrid", 3000, 1, 6.0),
            // Outside the range: ignored.
            transfer("2026-10-10", 9, "a.com", "direct", 99, 1, 1.0),
        ];
        let days = [day_row("2026-10-13", 1, 2, Some(500)), day_row("2026-10-15", 2, 0, Some(800))];
        let all_time = AllTimeRow { bytes: 4099, files: 4, failed: 2, since: Some(1_700_000_000) };
        let report = build_report(3, date(15), &transfers, &days, &all_time);

        let summary: Vec<_> =
            report.daily.iter().map(|d| (d.day.as_str(), d.bytes, d.files, d.failed, d.peak_speed)).collect();
        assert_eq!(
            summary,
            [("2026-10-13", 0, 0, 2, Some(500)), ("2026-10-14", 4000, 3, 0, None), ("2026-10-15", 0, 0, 0, Some(800)),]
        );
        assert_eq!(
            report.totals,
            Totals { bytes: 4000, files: 3, packages: 3, failed: 2, avg_speed: Some(400), peak_speed: Some(800) }
        );
        assert_eq!(report.all_time, AllTime { bytes: 4099, files: 4, since: Some(1_700_000_000) });
        assert_eq!(report.hours[9], 1000);
        assert_eq!(report.hours[21], 3000);
        assert_eq!(report.hours.iter().sum::<u64>(), 4000);
        assert_eq!(
            report.providers,
            [
                ProviderTotal { via: "alldebrid".into(), bytes: 3000, files: 1 },
                ProviderTotal { via: "direct".into(), bytes: 1000, files: 2 },
            ]
        );
    }

    #[test]
    fn empty_range_has_no_speeds() {
        let report = build_report(1, date(15), &[], &[], &AllTimeRow::default());
        assert_eq!(report.daily.len(), 1);
        assert_eq!(report.daily[0].day, "2026-10-15");
        assert_eq!(report.totals, Totals::default());
        assert!(report.hosts.is_empty() && report.providers.is_empty());
        assert_eq!(report.all_time.since, None);
    }

    #[test]
    fn groups_small_hosts_as_other() {
        let transfers: Vec<TransferRow> =
            (1..=10).map(|i| transfer("2026-10-15", 12, &format!("h{i:02}.com"), "direct", i * 100, 1, 1.0)).collect();
        let report = build_report(7, date(15), &transfers, &[], &AllTimeRow::default());
        let hosts: Vec<_> = report.hosts.iter().map(|h| (h.host.as_str(), h.bytes, h.files)).collect();
        assert_eq!(hosts.len(), TOP_HOSTS + 1);
        assert_eq!(hosts[0], ("h10.com", 1000, 1));
        assert_eq!(hosts[7], ("h03.com", 300, 1));
        assert_eq!(hosts[8], (OTHER_HOSTS, 300, 2));
    }

    /// A package with `files` downloads; returns the ids of the package and of the downloads.
    async fn package_with_files(db: &Db, files: usize) -> (i64, Vec<i64>) {
        let package = db::packages::NewPackage {
            name: "Package".into(),
            name_locked: true,
            category_id: None,
            create_subfolder: false,
            extract: false,
            delete_archives: false,
            archive_password: None,
            source_kind: crate::models::SourceKind::Links,
            source: None,
            stage: crate::models::Stage::Download,
            remote_account_id: None,
            remote_id: None,
            paused: false,
        };
        let id = db::packages::insert(db, &package).await.unwrap();
        let new: Vec<db::downloads::NewDownload> = (0..files)
            .map(|i| db::downloads::NewDownload {
                url: format!("https://example.com/{i}"),
                rel_dir: String::new(),
                filename: None,
                size: None,
                host: None,
                account_id: None,
            })
            .collect();
        db::downloads::insert_many(db, id, &new).await.unwrap();
        let rows = db::downloads::list_by_package(db, id).await.unwrap();
        (id, rows.iter().map(|r| r.id).collect())
    }

    #[tokio::test]
    async fn records_and_reports_from_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let db = db::open(&dir.path().join("rustload.db")).await.unwrap();
        let (package, files) = package_with_files(&db, 3).await;
        let recorder = StatsRecorder::new(db.clone());
        recorder.file_completed(files[0], Some("a.com"), Some("direct"), 1000, 500, Duration::from_secs(5)).await;
        recorder.file_completed(files[1], Some("a.com"), Some("direct"), 3000, 3000, Duration::from_secs(10)).await;
        recorder.file_completed(files[2], None, Some("realdebrid"), 4000, 4000, Duration::from_secs(20)).await;
        // Downloaded again (checksum mismatch) and extracted again: counted once.
        recorder.file_completed(files[1], Some("a.com"), Some("direct"), 3000, 3000, Duration::from_secs(10)).await;
        recorder.file_failed().await;
        recorder.package_finished(package).await;
        recorder.package_finished(package).await;
        recorder.sample_speed(700).await;
        recorder.sample_speed(900).await;
        recorder.flush().await;

        // A week-long range is immune to the day changing during the test.
        let report = report(&db, 7, Local::now().date_naive()).await.unwrap();
        assert_eq!(
            report.totals,
            Totals { bytes: 8000, files: 3, packages: 1, failed: 1, avg_speed: Some(200), peak_speed: Some(900) }
        );
        assert_eq!(report.all_time.files, 3);
        assert!(report.all_time.since.is_some());
        let hosts: Vec<_> = report.hosts.iter().map(|h| (h.host.as_str(), h.files)).collect();
        assert_eq!(hosts, [("", 1), ("a.com", 2)]);
        db.close().await;
    }

    #[test]
    fn scales_the_time_of_resumed_files() {
        assert_eq!(transfer_secs(1000, 1000, Duration::from_secs(10)), 10.0);
        assert_eq!(transfer_secs(1000, 250, Duration::from_secs(10)), 40.0);
        assert_eq!(transfer_secs(1000, 0, Duration::ZERO), 0.0);
    }

    #[test]
    fn writes_the_peak_when_it_rises_at_most_once_a_minute() {
        let start = Instant::now();
        let at = |secs| start + Duration::from_secs(secs);
        let mut peak = Peak::new(date(15));
        assert_eq!(peak.sample(date(15), 100, at(0)), [(date(15), 100)]);
        // Rises again within the minute: kept in memory.
        assert!(peak.sample(date(15), 300, at(10)).is_empty());
        assert!(peak.sample(date(15), 200, at(30)).is_empty());
        assert_eq!(peak.sample(date(15), 50, at(61)), [(date(15), 300)]);
        // Nothing new to write.
        assert!(peak.sample(date(15), 50, at(200)).is_empty());
        assert_eq!(peak.take_pending(), None);
        // A rise long after the last write is written at once.
        assert_eq!(peak.sample(date(15), 400, at(210)), [(date(15), 400)]);
        // The day changes with an unwritten peak: it is written for the previous day.
        assert!(peak.sample(date(15), 500, at(220)).is_empty());
        assert_eq!(peak.sample(date(16), 10, at(230)), [(date(15), 500), (date(16), 10)]);
    }
}
