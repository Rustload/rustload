//! User settings: one JSON document stored in the `kv` table and broadcast to the rest of the
//! application through a `watch` channel, so changes apply without restart.

use std::sync::Arc;

use anyhow::Result;
use chrono::{Datelike, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::db::{self, Db};

const KV_KEY: &str = "settings";
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub general: General,
    pub downloads: Downloads,
    pub schedule: Schedule,
    pub storage: Storage,
    pub postprocess: PostProcess,
    pub debrid: Debrid,
    pub watch: Watch,
    pub clicknload: ClickNLoad,
    pub notifications: Notifications,
    pub hooks: Hooks,
    pub integrations: Integrations,
    pub telegram_bot: TelegramBot,
    pub media: crate::media::MediaSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// `""` follows the browser language.
    pub language: String,
    pub theme: String,
}

impl Default for General {
    fn default() -> Self {
        Self { language: String::new(), theme: "system".into() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileExists {
    Rename,
    Overwrite,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Downloads {
    pub max_parallel: u32,
    pub connections: u32,
    pub min_split_size_mib: u32,
    pub speed_limit_enabled: bool,
    pub speed_limit_kib: u64,
    pub max_retries: u32,
    pub retry_delay_secs: u64,
    pub timeout_secs: u64,
    pub user_agent: String,
    pub file_exists: FileExists,
    pub auto_start: bool,
}

impl Default for Downloads {
    fn default() -> Self {
        Self {
            max_parallel: 3,
            connections: 8,
            min_split_size_mib: 4,
            speed_limit_enabled: false,
            speed_limit_kib: 10 * 1024,
            max_retries: 5,
            retry_delay_secs: 30,
            timeout_secs: 30,
            user_agent: DEFAULT_USER_AGENT.into(),
            file_exists: FileExists::Rename,
            auto_start: true,
        }
    }
}

impl Downloads {
    /// Global limit in bytes/s, `None` when unlimited.
    pub fn speed_limit(&self) -> Option<u64> {
        (self.speed_limit_enabled && self.speed_limit_kib > 0).then(|| self.speed_limit_kib * 1024)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    pub enabled: bool,
    pub start: String,
    pub end: String,
    /// Monday first.
    pub days: Vec<bool>,
    /// Lower speed limits during time windows (independent of `enabled`).
    pub speed_rules: Vec<SpeedRule>,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            enabled: false,
            start: "01:00".into(),
            end: "08:00".into(),
            days: vec![true; 7],
            speed_rules: Vec::new(),
        }
    }
}

impl Schedule {
    /// Whether downloads may run at `now` (local time).
    pub fn is_open(&self, now: chrono::NaiveDateTime) -> bool {
        !self.enabled || in_window(&self.start, &self.end, &self.days, now).unwrap_or(true)
    }
}

/// A speed limit applied during a time window, e.g. "1 MiB/s on weekdays from 08:00 to 23:00".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpeedRule {
    pub enabled: bool,
    pub start: String,
    pub end: String,
    /// Monday first.
    pub days: Vec<bool>,
    pub limit_kib: u64,
}

impl Default for SpeedRule {
    fn default() -> Self {
        Self { enabled: true, start: "08:00".into(), end: "23:00".into(), days: vec![true; 7], limit_kib: 1024 }
    }
}

impl SpeedRule {
    pub fn is_active(&self, now: chrono::NaiveDateTime) -> bool {
        self.enabled && in_window(&self.start, &self.end, &self.days, now).unwrap_or(false)
    }
}

/// Maximum number of speed rules.
const MAX_SPEED_RULES: usize = 10;

/// Whether `now` is inside the window from `start` to `end` ("HH:MM") on the enabled `days`
/// (Monday first); `None` when a time is invalid. Equal times cover the whole day. A window whose
/// end is before its start crosses midnight; the part after midnight belongs to the previous
/// day's window.
fn in_window(start: &str, end: &str, days: &[bool], now: chrono::NaiveDateTime) -> Option<bool> {
    let (start, end) = (parse_hhmm(start)?, parse_hhmm(end)?);
    let minute = now.hour() * 60 + now.minute();
    let weekday = now.weekday().num_days_from_monday() as usize;
    let day_enabled = |day: usize| days.get(day % 7).copied().unwrap_or(true);
    let start = start.hour() * 60 + start.minute();
    let end = end.hour() * 60 + end.minute();

    Some(if start == end {
        day_enabled(weekday)
    } else if start < end {
        day_enabled(weekday) && (start..end).contains(&minute)
    } else if minute >= start {
        day_enabled(weekday)
    } else {
        minute < end && day_enabled(weekday + 6)
    })
}

fn parse_hhmm(value: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(value, "%H:%M").ok()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Storage {
    pub create_subfolder: bool,
    /// Downloads pause while the download root has less free space than this; 0 disables.
    pub min_free_mib: u64,
}

impl Default for Storage {
    fn default() -> Self {
        Self { create_subfolder: true, min_free_mib: 1024 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PostProcess {
    pub extract: bool,
    pub delete_archives: bool,
    pub passwords: Vec<String>,
    /// Check files against checksum files and provider hashes before extraction.
    pub verify_checksums: bool,
}

impl Default for PostProcess {
    fn default() -> Self {
        Self { extract: true, delete_archives: true, passwords: Vec::new(), verify_checksums: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Debrid {
    pub enabled: bool,
    pub delete_magnets: bool,
}

impl Default for Debrid {
    fn default() -> Self {
        Self { enabled: true, delete_magnets: false }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Watch {
    pub enabled: bool,
    pub dir: String,
    pub category_id: Option<i64>,
}

impl Default for Watch {
    fn default() -> Self {
        Self { enabled: false, dir: "/watch".into(), category_id: None }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClickNLoad {
    pub enabled: bool,
    /// Received links wait for a confirmation (resume) instead of starting right away.
    pub paused: bool,
}

impl Default for ClickNLoad {
    fn default() -> Self {
        Self { enabled: false, paused: true }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub targets: Vec<NotificationTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationKind {
    Webhook,
    Discord,
    Telegram,
    Ntfy,
    Gotify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationEvent {
    PackageFinished,
    PackageFailed,
    QueueFinished,
    ExtractFailed,
    ChecksumFailed,
    DiskLow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationTarget {
    pub id: String,
    pub name: String,
    pub kind: NotificationKind,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub chat_id: String,
    #[serde(default)]
    pub events: Vec<NotificationEvent>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hooks {
    pub package_finished: String,
    pub package_failed: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Integrations {
    pub api_key: String,
    /// Serve Prometheus metrics on `/metrics`.
    pub metrics: bool,
}

/// Telegram bot receiving links and commands (see `integrations::telegram`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramBot {
    pub enabled: bool,
    /// Token given by @BotFather.
    pub token: String,
    /// Chat ids the bot obeys.
    pub allowed_chats: Vec<String>,
}

impl TelegramBot {
    fn validate(&mut self) -> Result<(), String> {
        self.token = self.token.trim().to_owned();
        if self.enabled && self.token.is_empty() {
            return Err("telegram_bot.token is required when the bot is enabled".into());
        }
        if !self.token.is_empty() && !is_bot_token(&self.token) {
            return Err("telegram_bot.token is not a bot token (it looks like `123456789:AAE...`)".into());
        }
        let mut chats: Vec<String> = Vec::new();
        for chat in self.allowed_chats.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
            if chat.parse::<i64>().is_err() {
                return Err(format!("telegram_bot.allowed_chats: `{chat}` is not a chat id (a number)"));
            }
            if !chats.iter().any(|c| c == chat) {
                chats.push(chat.to_owned());
            }
        }
        self.allowed_chats = chats;
        Ok(())
    }
}

/// `<bot id>:<secret>` as given by @BotFather. The token is part of the API URLs, so nothing else
/// is accepted.
pub fn is_bot_token(token: &str) -> bool {
    token.split_once(':').is_some_and(|(id, secret)| {
        !id.is_empty()
            && id.bytes().all(|b| b.is_ascii_digit())
            && !secret.is_empty()
            && secret.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    })
}

impl Settings {
    /// Speed limit in bytes/s at `now` (local time), `None` when unlimited: the lowest of the
    /// global limit and of the active speed rules. The flag tells whether a rule is active.
    pub fn speed_limit_at(&self, now: chrono::NaiveDateTime) -> (Option<u64>, bool) {
        let rules = self.schedule.speed_rules.iter().filter(|rule| rule.is_active(now));
        let rule_limit = rules.map(|rule| rule.limit_kib.saturating_mul(1024)).min();
        let limit = match (self.downloads.speed_limit(), rule_limit) {
            (Some(global), Some(rule)) => Some(global.min(rule)),
            (global, rule) => global.or(rule),
        };
        (limit, rule_limit.is_some())
    }

    /// Normalises values and rejects inconsistent ones with a user-facing message.
    pub fn validate(mut self) -> Result<Self, String> {
        let d = &mut self.downloads;
        check_range("downloads.max_parallel", d.max_parallel, 1, 20)?;
        check_range("downloads.connections", d.connections, 1, 32)?;
        check_range("downloads.min_split_size_mib", d.min_split_size_mib, 1, 64)?;
        check_range("downloads.max_retries", d.max_retries, 0, 50)?;
        check_range("downloads.retry_delay_secs", d.retry_delay_secs, 1, 86_400)?;
        check_range("downloads.timeout_secs", d.timeout_secs, 5, 600)?;
        if d.speed_limit_enabled && d.speed_limit_kib == 0 {
            return Err("downloads.speed_limit_kib must be greater than 0 when the limit is enabled".into());
        }
        d.user_agent = d.user_agent.trim().to_owned();
        if d.user_agent.is_empty() {
            d.user_agent = DEFAULT_USER_AGENT.into();
        }

        check_range("storage.min_free_mib", self.storage.min_free_mib, 0, 1024 * 1024)?;

        let s = &self.schedule;
        if parse_hhmm(&s.start).is_none() || parse_hhmm(&s.end).is_none() {
            return Err("schedule.start and schedule.end must use the HH:MM format".into());
        }
        if s.days.len() != 7 {
            return Err("schedule.days must contain 7 entries".into());
        }
        if s.speed_rules.len() > MAX_SPEED_RULES {
            return Err(format!("schedule.speed_rules accepts at most {MAX_SPEED_RULES} rules"));
        }
        for (index, rule) in s.speed_rules.iter().enumerate() {
            let name = format!("schedule.speed_rules[{index}]");
            if parse_hhmm(&rule.start).is_none() || parse_hhmm(&rule.end).is_none() {
                return Err(format!("{name}: start and end must use the HH:MM format"));
            }
            if rule.days.len() != 7 {
                return Err(format!("{name}: days must contain 7 entries"));
            }
            if rule.limit_kib == 0 {
                return Err(format!("{name}: limit_kib must be greater than 0"));
            }
        }

        // A language tag such as `fr` or `pt-BR`; the interface decides which ones it ships.
        let language = &self.general.language;
        if language.len() > 10 || !language.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
            return Err("general.language must be a language tag such as `en` or `fr` (empty = automatic)".into());
        }
        if !matches!(self.general.theme.as_str(), "dark" | "light" | "system") {
            return Err("general.theme must be `dark`, `light` or `system`".into());
        }

        self.postprocess.passwords =
            self.postprocess.passwords.iter().map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect();
        self.watch.dir = self.watch.dir.trim().to_owned();
        if self.watch.enabled && self.watch.dir.is_empty() {
            return Err("watch.dir is required when the watch folder is enabled".into());
        }

        for target in &mut self.notifications.targets {
            target.url = target.url.trim().to_owned();
            target.token = target.token.trim().to_owned();
            target.chat_id = target.chat_id.trim().to_owned();
            let missing = match target.kind {
                NotificationKind::Telegram => target.token.is_empty() || target.chat_id.is_empty(),
                NotificationKind::Gotify => target.url.is_empty() || target.token.is_empty(),
                _ => target.url.is_empty(),
            };
            if missing {
                return Err(format!("notification `{}` is missing required fields", target.name));
            }
        }
        self.hooks.package_finished = self.hooks.package_finished.trim().to_owned();
        self.hooks.package_failed = self.hooks.package_failed.trim().to_owned();
        self.telegram_bot.validate()?;
        self.media.validate()?;
        Ok(self)
    }
}

fn check_range<T: PartialOrd + std::fmt::Display>(name: &str, value: T, min: T, max: T) -> Result<(), String> {
    if value < min || value > max {
        return Err(format!("{name} must be between {min} and {max}"));
    }
    Ok(())
}

/// Why [`SettingsStore::replace`] failed.
#[derive(Debug, thiserror::Error)]
pub enum ReplaceError {
    /// The settings were refused: the message says why.
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

/// Shared, live-updating access to the settings.
pub struct SettingsStore {
    db: Db,
    tx: watch::Sender<Arc<Settings>>,
    /// Serialises read-modify-write cycles.
    write_lock: tokio::sync::Mutex<()>,
}

impl SettingsStore {
    pub async fn load(db: Db) -> Result<Self> {
        let mut settings: Settings = match db::kv_get(&db, KV_KEY).await? {
            Some(raw) => serde_json::from_str(&raw).unwrap_or_else(|error| {
                tracing::warn!("settings could not be parsed ({error}); using defaults");
                Settings::default()
            }),
            None => Settings::default(),
        };
        if settings.integrations.api_key.is_empty() {
            settings.integrations.api_key = crate::util::random_token();
        }
        db::kv_set(&db, KV_KEY, &serde_json::to_string(&settings)?).await?;
        Ok(Self { db, tx: watch::channel(Arc::new(settings)).0, write_lock: Default::default() })
    }

    pub fn get(&self) -> Arc<Settings> {
        self.tx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<Settings>> {
        self.tx.subscribe()
    }

    /// Replaces the settings coming from the UI. The API key can only change through
    /// [`Self::regenerate_api_key`].
    pub async fn replace(&self, new: Settings) -> Result<Arc<Settings>, ReplaceError> {
        let _guard = self.write_lock.lock().await;
        let mut new = new.validate().map_err(ReplaceError::Invalid)?;
        new.integrations.api_key = self.get().integrations.api_key.clone();
        Ok(self.store(new).await?)
    }

    /// Applies an internal modification (no validation needed).
    pub async fn modify(&self, change: impl FnOnce(&mut Settings)) -> Result<Arc<Settings>> {
        let _guard = self.write_lock.lock().await;
        let mut settings = (*self.get()).clone();
        change(&mut settings);
        self.store(settings).await
    }

    pub async fn regenerate_api_key(&self) -> Result<String> {
        let key = crate::util::random_token();
        let stored = key.clone();
        self.modify(move |s| s.integrations.api_key = stored).await?;
        Ok(key)
    }

    async fn store(&self, settings: Settings) -> Result<Arc<Settings>> {
        db::kv_set(&self.db, KV_KEY, &serde_json::to_string(&settings)?).await?;
        let settings = Arc::new(settings);
        self.tx.send_replace(settings.clone());
        Ok(settings)
    }

    /// Starts replacing the settings together with other database changes (backup restore).
    /// Other settings changes wait until the transaction is committed or dropped.
    pub async fn begin(&self) -> Result<SettingsTransaction<'_>> {
        let guard = self.write_lock.lock().await;
        Ok(SettingsTransaction { store: self, _guard: guard, tx: db::begin_write(&self.db).await? })
    }
}

/// See [`SettingsStore::begin`]. Dropping it rolls everything back.
pub struct SettingsTransaction<'a> {
    store: &'a SettingsStore,
    _guard: tokio::sync::MutexGuard<'a, ()>,
    pub tx: sqlx::SqliteTransaction<'static>,
}

impl SettingsTransaction<'_> {
    /// Stores `settings` (already validated), commits the transaction, then publishes them.
    pub async fn commit(mut self, settings: Settings) -> Result<Arc<Settings>> {
        db::kv_set(&mut *self.tx, KV_KEY, &serde_json::to_string(&settings)?).await?;
        self.tx.commit().await?;
        let settings = Arc::new(settings);
        self.store.tx.send_replace(settings.clone());
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn at(day: u32, hour: u32, minute: u32) -> chrono::NaiveDateTime {
        // 2026-10-12 is a Monday.
        NaiveDate::from_ymd_opt(2026, 10, 12 + day).unwrap().and_hms_opt(hour, minute, 0).unwrap()
    }

    fn schedule(start: &str, end: &str, days: [bool; 7]) -> Schedule {
        Schedule { enabled: true, start: start.into(), end: end.into(), days: days.to_vec(), ..Default::default() }
    }

    fn rule(start: &str, end: &str, days: [bool; 7], limit_kib: u64) -> SpeedRule {
        SpeedRule { enabled: true, start: start.into(), end: end.into(), days: days.to_vec(), limit_kib }
    }

    const WEEKDAYS: [bool; 7] = [true, true, true, true, true, false, false];
    const FRIDAY: [bool; 7] = [false, false, false, false, true, false, false];

    #[test]
    fn speed_rules_apply_inside_their_window() {
        let mut settings = Settings::default();
        settings.schedule.speed_rules =
            vec![rule("08:00", "23:00", WEEKDAYS, 1024), rule("23:00", "01:00", FRIDAY, 4096)];
        assert_eq!(settings.speed_limit_at(at(0, 12, 0)), (Some(1024 * 1024), true));
        assert_eq!(settings.speed_limit_at(at(0, 7, 59)), (None, false));
        assert_eq!(settings.speed_limit_at(at(0, 23, 0)), (None, false));
        assert_eq!(settings.speed_limit_at(at(5, 12, 0)), (None, false));
        // Friday night, past midnight included.
        assert_eq!(settings.speed_limit_at(at(4, 23, 30)), (Some(4096 * 1024), true));
        assert_eq!(settings.speed_limit_at(at(5, 0, 30)), (Some(4096 * 1024), true));
        assert_eq!(settings.speed_limit_at(at(5, 23, 30)), (None, false));
        // The schedule window itself does not matter.
        settings.schedule.enabled = true;
        assert_eq!(settings.speed_limit_at(at(0, 12, 0)), (Some(1024 * 1024), true));
    }

    #[test]
    fn lowest_limit_wins() {
        let mut settings = Settings::default();
        settings.downloads.speed_limit_enabled = true;
        settings.downloads.speed_limit_kib = 2048;
        settings.schedule.speed_rules =
            vec![rule("08:00", "08:00", [true; 7], 4096), rule("12:00", "14:00", [true; 7], 512)];
        // Equal times cover the whole day.
        assert_eq!(settings.speed_limit_at(at(2, 9, 0)), (Some(2048 * 1024), true));
        assert_eq!(settings.speed_limit_at(at(2, 13, 0)), (Some(512 * 1024), true));
        settings.schedule.speed_rules[1].enabled = false;
        assert_eq!(settings.speed_limit_at(at(2, 13, 0)), (Some(2048 * 1024), true));
        settings.schedule.speed_rules.clear();
        assert_eq!(settings.speed_limit_at(at(2, 13, 0)), (Some(2048 * 1024), false));
        settings.downloads.speed_limit_enabled = false;
        assert_eq!(settings.speed_limit_at(at(2, 13, 0)), (None, false));
    }

    #[test]
    fn validates_speed_rules() {
        let with_rules = |rules: Vec<SpeedRule>| {
            let mut settings = Settings::default();
            settings.schedule.speed_rules = rules;
            settings.validate()
        };
        assert!(with_rules(vec![SpeedRule::default(); 10]).is_ok());
        assert!(with_rules(vec![SpeedRule::default(); 11]).is_err());
        assert!(with_rules(vec![SpeedRule { limit_kib: 0, ..Default::default() }]).is_err());
        assert!(with_rules(vec![SpeedRule { end: "8h".into(), ..Default::default() }]).is_err());
        assert!(with_rules(vec![SpeedRule { days: vec![true; 6], ..Default::default() }]).is_err());
        // A rule sent without some fields gets the defaults.
        let partial: SpeedRule = serde_json::from_str(r#"{"start":"09:30","limit_kib":300}"#).unwrap();
        assert_eq!((partial.end.as_str(), partial.days.len(), partial.enabled), ("23:00", 7, true));
    }

    #[test]
    fn same_day_window() {
        let s = schedule("09:00", "17:00", [true; 7]);
        assert!(s.is_open(at(0, 9, 0)));
        assert!(s.is_open(at(0, 16, 59)));
        assert!(!s.is_open(at(0, 17, 0)));
        assert!(!s.is_open(at(0, 8, 59)));
    }

    #[test]
    fn overnight_window_belongs_to_start_day() {
        // Only Friday night is enabled: Friday 23:00 → Saturday 07:00.
        let s = schedule("23:00", "07:00", [false, false, false, false, true, false, false]);
        assert!(s.is_open(at(4, 23, 30)));
        assert!(s.is_open(at(5, 6, 0)));
        assert!(!s.is_open(at(5, 23, 30)));
        assert!(!s.is_open(at(4, 6, 0)));
    }

    #[test]
    fn disabled_schedule_is_always_open() {
        assert!(Schedule::default().is_open(at(2, 12, 0)));
    }

    #[test]
    fn older_documents_get_the_new_defaults() {
        let old =
            r#"{"storage":{"create_subfolder":false},"postprocess":{"extract":false},"schedule":{"enabled":true}}"#;
        let settings: Settings = serde_json::from_str(old).unwrap();
        assert!(!settings.storage.create_subfolder);
        assert_eq!(settings.storage.min_free_mib, 1024);
        assert!(settings.postprocess.verify_checksums);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn validation_rejects_bad_values() {
        let mut settings = Settings::default();
        settings.downloads.connections = 0;
        assert!(settings.validate().is_err());

        let mut settings = Settings::default();
        settings.schedule.start = "25:00".into();
        assert!(settings.validate().is_err());

        let mut settings = Settings::default();
        settings.storage.min_free_mib = 2 * 1024 * 1024;
        assert!(settings.validate().is_err());

        assert!(Settings::default().validate().is_ok());
    }

    #[test]
    fn validates_the_telegram_bot() {
        assert!(is_bot_token("123456789:AAE-x_yZ09"));
        for token in ["", "123456789", ":abc", "12a:abc", "123:", "123:abc/../x", "123:abc def"] {
            assert!(!is_bot_token(token), "{token}");
        }

        let mut settings = Settings::default();
        settings.telegram_bot.enabled = true;
        assert!(settings.clone().validate().is_err());
        settings.telegram_bot.token = " 123:abc ".into();
        settings.telegram_bot.allowed_chats = vec![" 42 ".into(), "".into(), "-1001234".into(), "42".into()];
        let valid = settings.clone().validate().unwrap();
        assert_eq!(valid.telegram_bot.token, "123:abc");
        assert_eq!(valid.telegram_bot.allowed_chats, ["42", "-1001234"]);
        settings.telegram_bot.allowed_chats = vec!["@me".into()];
        assert!(settings.validate().is_err());
    }
}
