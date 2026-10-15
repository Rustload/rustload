//! In-process event bus feeding the `/api/events` Server-Sent Events stream.

use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

/// One SSE message: the event name and its JSON payload.
#[derive(Debug, Clone)]
pub struct Event {
    pub name: &'static str,
    pub data: Value,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    Success,
    Info,
    Warning,
    Error,
}

/// Toast-worthy message. `key` + `params` let the UI translate it; `message` is the English text.
#[derive(Debug, Clone, Serialize)]
pub struct Notice {
    pub level: NoticeLevel,
    pub key: &'static str,
    pub params: BTreeMap<&'static str, String>,
    pub message: String,
}

impl Notice {
    pub fn new(level: NoticeLevel, key: &'static str, message: impl Into<String>) -> Self {
        Self { level, key, params: BTreeMap::new(), message: message.into() }
    }

    pub fn param(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.params.insert(name, value.into());
        self
    }
}

pub struct EventBus {
    tx: broadcast::Sender<Event>,
    queue_dirty: AtomicBool,
    history_dirty: AtomicBool,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            tx: broadcast::channel(512).0,
            queue_dirty: AtomicBool::new(false),
            history_dirty: AtomicBool::new(false),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    pub fn emit(&self, name: &'static str, data: impl Serialize) {
        let data = serde_json::to_value(data).unwrap_or(Value::Null);
        // Sending only fails when nobody listens, which is fine.
        let _ = self.tx.send(Event { name, data });
    }

    pub fn notice(&self, notice: Notice) {
        self.emit("notice", notice);
    }

    /// Marks the queue as changed; coalesced into one `queue` event per flush interval.
    pub fn queue_changed(&self) {
        self.queue_dirty.store(true, Ordering::Relaxed);
    }

    pub fn history_changed(&self) {
        self.history_dirty.store(true, Ordering::Relaxed);
    }

    /// Emits coalesced `queue`/`history` events until `shutdown` fires.
    pub async fn run_flusher(&self, shutdown: CancellationToken) {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = interval.tick() => {}
            }
            if self.queue_dirty.swap(false, Ordering::Relaxed) {
                self.emit("queue", serde_json::json!({}));
            }
            if self.history_dirty.swap(false, Ordering::Relaxed) {
                self.emit("history", serde_json::json!({}));
            }
        }
    }
}
