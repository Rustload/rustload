//! Logging: stdout, a daily-rotated file in `<config>/logs`, and an in-memory ring buffer that
//! feeds the web UI (history + live stream).

use std::{
    collections::VecDeque,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::sync::broadcast;
use tracing::{Event, Level, Subscriber, field::Field};
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{self, Rotation},
};
use tracing_subscriber::{EnvFilter, Layer, layer::Context as LayerContext, prelude::*};

const BUFFER_CAPACITY: usize = 5_000;
const LOG_FILE_PREFIX: &str = "rustload";
/// Dependencies are noisy at `info`; the user's filter is appended so it can override these.
const QUIET_DEPENDENCIES: &str =
    "hyper=warn,hyper_util=warn,h2=warn,rustls=warn,reqwest=warn,sqlx=warn,tower_http=warn";

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub id: u64,
    pub ts_ms: i64,
    pub level: &'static str,
    pub target: String,
    pub message: String,
}

/// Bounded in-memory log history with a broadcast channel for live subscribers.
pub struct LogBuffer {
    entries: Mutex<VecDeque<LogEntry>>,
    next_id: AtomicU64,
    live: broadcast::Sender<LogEntry>,
}

impl LogBuffer {
    fn new() -> Self {
        Self {
            entries: Mutex::new(VecDeque::with_capacity(BUFFER_CAPACITY)),
            next_id: AtomicU64::new(1),
            live: broadcast::channel(256).0,
        }
    }

    fn push(&self, level: Level, target: &str, message: String) {
        let entry = LogEntry {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            ts_ms: chrono::Utc::now().timestamp_millis(),
            level: level_name(level),
            target: target.strip_prefix("rustload::").unwrap_or(target).to_owned(),
            message,
        };
        {
            let mut entries = self.entries.lock().expect("log buffer poisoned");
            if entries.len() == BUFFER_CAPACITY {
                entries.pop_front();
            }
            entries.push_back(entry.clone());
        }
        // No receivers is fine: nobody is watching the logs page.
        let _ = self.live.send(entry);
    }

    /// Latest `limit` entries at or above `min_level`, oldest first.
    pub fn snapshot(&self, min_level: Level, limit: usize) -> Vec<LogEntry> {
        let entries = self.entries.lock().expect("log buffer poisoned");
        let mut selected: Vec<LogEntry> =
            entries.iter().rev().filter(|e| level_from_name(e.level) <= min_level).take(limit).cloned().collect();
        selected.reverse();
        selected
    }

    pub fn clear(&self) {
        self.entries.lock().expect("log buffer poisoned").clear();
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.live.subscribe()
    }
}

pub fn level_name(level: Level) -> &'static str {
    match level {
        Level::ERROR => "ERROR",
        Level::WARN => "WARN",
        Level::INFO => "INFO",
        Level::DEBUG => "DEBUG",
        Level::TRACE => "TRACE",
    }
}

pub fn level_from_name(name: &str) -> Level {
    match name.to_ascii_uppercase().as_str() {
        "ERROR" => Level::ERROR,
        "WARN" | "WARNING" => Level::WARN,
        "DEBUG" => Level::DEBUG,
        "TRACE" => Level::TRACE,
        _ => Level::INFO,
    }
}

/// `tracing` layer copying every event into the [`LogBuffer`].
struct BufferLayer(Arc<LogBuffer>);

impl<S: Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: LayerContext<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let meta = event.metadata();
        self.0.push(*meta.level(), meta.target(), visitor.finish());
    }
}

/// Renders `message` followed by the other fields as `key=value`.
#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: String,
}

impl MessageVisitor {
    fn finish(self) -> String {
        if self.fields.is_empty() {
            self.message
        } else if self.message.is_empty() {
            self.fields
        } else {
            format!("{} {}", self.message, self.fields)
        }
    }
}

impl tracing::field::Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.record_debug(field, &value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            let _ = write!(self.fields, "{}={value:?}", field.name());
        }
    }
}

/// Installs the global subscriber. Keep the returned guard alive for the whole process so the
/// file writer flushes on exit.
pub fn init(filter: &str, logs_dir: &Path) -> Result<(Arc<LogBuffer>, WorkerGuard)> {
    std::fs::create_dir_all(logs_dir).with_context(|| format!("cannot create log directory {}", logs_dir.display()))?;

    let filter = EnvFilter::try_new(format!("{QUIET_DEPENDENCIES},{filter}"))
        .with_context(|| format!("invalid log filter `{filter}`"))?;

    let file_appender = rolling::Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix("log")
        .max_log_files(7)
        .build(logs_dir)
        .context("cannot open log file")?;
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let buffer = Arc::new(LogBuffer::new());
    // Only the first server of a process logs (tests start several).
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .with(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(file_writer))
        .with(BufferLayer(buffer.clone()))
        .try_init();

    Ok((buffer, guard))
}

/// Most recently modified log file, served by `GET /api/logs/file`.
pub fn latest_log_file(logs_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(logs_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(LOG_FILE_PREFIX))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}
