//! Telegram bot: add links, magnets and `.torrent` files, and control the queue from a chat.
//!
//! The bot long-polls `getUpdates`, so it needs no public address. It only obeys the chats listed
//! in the settings; other chats get one answer with their id, so that the user can allow them.
//! The token is part of every Bot API URL: errors are stripped of their URL before being shown.
//!
//! Messages adding links or torrents are handled one at a time, in the order received (a playlist
//! takes up to a minute to list), while commands are answered at once, next to them.

use std::{
    collections::HashMap,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    db,
    engine::{
        Engine,
        intake::{self, AddOptions},
    },
    events::{Notice, NoticeLevel},
    models::{DownloadRow, PackageRow, PackageStatus, SourceKind, Stage},
    stats::Snapshot,
    util::human_bytes,
};

const API: &str = "https://api.telegram.org";
/// How long `getUpdates` waits for messages, seconds.
const POLL_TIMEOUT: u64 = 50;
/// Must exceed the long-polling wait.
const HTTP_TIMEOUT: Duration = Duration::from_secs(POLL_TIMEOUT + 20);
const MIN_BACKOFF: Duration = Duration::from_secs(2);
const MAX_BACKOFF: Duration = Duration::from_secs(300);
/// Chats that are not allowed get at most one answer per interval.
const REFUSAL_INTERVAL: Duration = Duration::from_secs(600);
const MAX_TORRENT_SIZE: usize = 10 * 1024 * 1024;
/// Packages listed by `/queue`.
const QUEUE_LINES: usize = 10;
/// Messages adding links or torrents that may wait for their turn.
const ADD_QUEUE: usize = 32;
/// Commands answered at the same time.
const MAX_COMMANDS: usize = 4;
const NAME_MAX_CHARS: usize = 60;

#[derive(Debug, thiserror::Error)]
pub enum BotError {
    #[error("network error: {0}")]
    Network(String),
    /// Reported by the Bot API, e.g. `Unauthorized`.
    #[error("{description}")]
    Api { description: String, retry_after: Option<u64> },
    #[error("{0}")]
    Other(String),
}

impl From<reqwest::Error> for BotError {
    fn from(error: reqwest::Error) -> Self {
        // The URL contains the token.
        Self::Network(error.without_url().to_string())
    }
}

/// Envelope of every Bot API answer.
#[derive(Deserialize)]
struct Reply<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
    parameters: Option<ReplyParameters>,
}

#[derive(Deserialize)]
struct ReplyParameters {
    retry_after: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct Message {
    chat: Chat,
    text: Option<String>,
    caption: Option<String>,
    document: Option<Document>,
}

#[derive(Debug, Deserialize)]
struct Chat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct Document {
    file_id: String,
    file_name: Option<String>,
    mime_type: Option<String>,
    file_size: Option<u64>,
}

#[derive(Deserialize)]
struct TelegramFile {
    file_path: Option<String>,
}

#[derive(Deserialize)]
struct User {
    username: Option<String>,
}

/// Bot API client for one token.
struct Bot {
    http: reqwest::Client,
    /// [`API`], except in tests.
    api: String,
    token: String,
}

impl Bot {
    async fn call<T: DeserializeOwned>(&self, method: &str, params: &Value) -> Result<T, BotError> {
        let response = self.http.post(format!("{}/bot{}/{method}", self.api, self.token)).json(params).send().await?;
        let status = response.status();
        let reply: Reply<T> = response
            .json()
            .await
            .map_err(|_| BotError::Other(format!("unexpected answer from Telegram (HTTP {status})")))?;
        match reply {
            Reply { ok: true, result: Some(result), .. } => Ok(result),
            reply => Err(BotError::Api {
                description: reply.description.unwrap_or_else(|| format!("HTTP {status}")),
                retry_after: reply.parameters.and_then(|p| p.retry_after),
            }),
        }
    }

    /// Username of the bot.
    async fn username(&self) -> Result<String, BotError> {
        let user: User = self.call("getMe", &json!({})).await?;
        Ok(user.username.unwrap_or_default())
    }

    /// Waits for new messages. Asking from `offset` also confirms the updates before it.
    async fn updates(&self, offset: i64) -> Result<Vec<Value>, BotError> {
        let params = json!({ "offset": offset, "timeout": POLL_TIMEOUT, "allowed_updates": ["message"] });
        self.call("getUpdates", &params).await
    }

    /// Confirms the updates before `offset` without waiting.
    async fn acknowledge(&self, offset: i64) -> Result<(), BotError> {
        self.call::<Vec<Value>>("getUpdates", &json!({ "offset": offset, "timeout": 0, "limit": 1 })).await.map(|_| ())
    }

    async fn download(&self, file_id: &str, max: usize) -> Result<Vec<u8>, BotError> {
        let file: TelegramFile = self.call("getFile", &json!({ "file_id": file_id })).await?;
        let path = file.file_path.ok_or_else(|| BotError::Other("Telegram did not provide the file".into()))?;
        let mut response =
            self.http.get(format!("{}/file/bot{}/{path}", self.api, self.token)).send().await?.error_for_status()?;
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if data.len() + chunk.len() > max {
                return Err(BotError::Other("the file is too large".into()));
            }
            data.extend_from_slice(&chunk);
        }
        Ok(data)
    }

    /// Sends a MarkdownV2 message; failures are only logged.
    async fn send(&self, chat: i64, text: &str) {
        let params = json!({
            "chat_id": chat,
            "text": text,
            "parse_mode": "MarkdownV2",
            "link_preview_options": { "is_disabled": true },
        });
        if let Err(error) = self.call::<Value>("sendMessage", &params).await {
            tracing::warn!("Telegram bot: cannot answer: {error}");
        }
    }
}

/// Username of the bot behind `token` (`getMe`), to check a token.
pub async fn bot_username(http: &reqwest::Client, token: &str) -> Result<String, BotError> {
    Bot { http: http.clone(), api: API.into(), token: token.to_owned() }.username().await
}

/// Runs the bot while it is enabled, restarting it when its token changes.
pub async fn supervise(engine: Arc<Engine>) {
    let http = match reqwest::Client::builder()
        .user_agent(concat!("Rustload/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(HTTP_TIMEOUT)
        .build()
    {
        Ok(http) => http,
        Err(error) => {
            tracing::error!("Telegram bot unavailable: {error}");
            return;
        }
    };
    let mut settings = engine.settings.subscribe();
    let mut running: Option<(String, CancellationToken)> = None;
    loop {
        let wanted = {
            let current = settings.borrow_and_update();
            let bot = &current.telegram_bot;
            (bot.enabled && !bot.token.is_empty()).then(|| bot.token.clone())
        };
        if running.as_ref().map(|(token, _)| token) != wanted.as_ref() {
            if let Some((_, stop)) = running.take() {
                stop.cancel();
            }
            if let Some(token) = wanted {
                let stop = engine.shutdown_token().child_token();
                let bot = Bot { http: http.clone(), api: API.into(), token: token.clone() };
                tokio::spawn(run(engine.clone(), bot, stop.clone()));
                running = Some((token, stop));
            }
        }
        tokio::select! {
            _ = engine.shutdown_token().cancelled() => return,
            changed = settings.changed() => if changed.is_err() { return },
        }
    }
}

async fn run(engine: Arc<Engine>, bot: Bot, stop: CancellationToken) {
    let bot = Arc::new(bot);
    let username = tokio::select! {
        _ = stop.cancelled() => return,
        username = bot.username() => username,
    };
    match username {
        Ok(username) => tracing::info!("Telegram bot @{username} started"),
        Err(error) => tracing::warn!("Telegram bot: {error}"),
    }

    let mut offset = 0;
    // Handled updates that Telegram was not told about yet.
    let mut unconfirmed = false;
    let mut backoff = MIN_BACKOFF;
    let mut refusals = HashMap::new();
    let tasks = TaskTracker::new();
    let (adds, queue) = mpsc::channel(ADD_QUEUE);
    tasks.spawn(add_in_order(engine.clone(), bot.clone(), queue));
    let workers = Workers { tasks: &tasks, adds, commands: Arc::new(Semaphore::new(MAX_COMMANDS)) };
    loop {
        let result = tokio::select! {
            _ = stop.cancelled() => break,
            result = bot.updates(offset) => result,
        };
        match result {
            Ok(updates) => {
                backoff = MIN_BACKOFF;
                unconfirmed = false;
                for update in updates {
                    let Some(id) = update.get("update_id").and_then(Value::as_i64) else { continue };
                    offset = offset.max(id + 1);
                    unconfirmed = true;
                    // Updates are parsed one by one: an unexpected one must not block the others.
                    let message = update.get("message").and_then(|m| Message::deserialize(m).ok());
                    if let Some(message) = message {
                        dispatch(&engine, &bot, message, &mut refusals, &workers).await;
                    }
                }
            }
            Err(error) => {
                let delay = match &error {
                    BotError::Api { retry_after: Some(secs), .. } => Duration::from_secs(*secs),
                    _ => backoff,
                };
                tracing::warn!("Telegram bot: {error} (retrying in {} s)", delay.as_secs());
                backoff = (backoff * 2).min(MAX_BACKOFF);
                tokio::select! {
                    _ = stop.cancelled() => break,
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        }
    }
    if unconfirmed {
        // Otherwise the last messages would be handled again on the next start.
        let _ = tokio::time::timeout(Duration::from_secs(5), bot.acknowledge(offset)).await;
    }
    // Messages received are still answered.
    drop(workers);
    tasks.close();
    tasks.wait().await;
    tracing::info!("Telegram bot stopped");
}

/// Where the messages of allowed chats are handled, see the module documentation.
struct Workers<'a> {
    tasks: &'a TaskTracker,
    /// Messages adding links or torrents, for [`add_in_order`].
    adds: mpsc::Sender<Message>,
    commands: Arc<Semaphore>,
}

async fn dispatch(
    engine: &Arc<Engine>,
    bot: &Arc<Bot>,
    message: Message,
    refusals: &mut HashMap<i64, Instant>,
    workers: &Workers<'_>,
) {
    let chat = message.chat.id;
    if !is_allowed(&engine.settings.get().telegram_bot.allowed_chats, chat) {
        if should_answer_refusal(refusals, chat, Instant::now()) {
            tracing::info!("Telegram bot: chat {chat} is not allowed");
            bot.send(chat, &refusal(chat)).await;
        }
        return;
    }
    if is_command(&message) {
        // Bounded: waits here when too many commands are being answered.
        let Ok(permit) = workers.commands.clone().acquire_owned().await else { return };
        let (engine, bot) = (engine.clone(), bot.clone());
        workers.tasks.spawn(async move {
            let answer = respond(&engine, &bot, &message).await;
            bot.send(chat, &answer).await;
            drop(permit);
        });
    } else if let Err(mpsc::error::TrySendError::Full(_)) = workers.adds.try_send(message) {
        bot.send(chat, &escape("Too many messages are waiting to be added: send this one again in a moment.")).await;
    }
}

/// Adds the links and torrents of the messages one at a time, so that packages and answers keep
/// the order of the messages. Ends when the bot stops, once the queue is empty.
async fn add_in_order(engine: Arc<Engine>, bot: Arc<Bot>, mut queue: mpsc::Receiver<Message>) {
    while let Some(message) = queue.recv().await {
        let answer = respond(&engine, &bot, &message).await;
        bot.send(message.chat.id, &answer).await;
    }
}

/// Whether a message is a command (`/status`...), as opposed to links or a torrent to add.
fn is_command(message: &Message) -> bool {
    message.document.is_none()
        && command(message.text.as_deref().or(message.caption.as_deref()).unwrap_or_default()).is_some()
}

fn is_allowed(allowed: &[String], chat: i64) -> bool {
    let chat = chat.to_string();
    allowed.iter().any(|c| c.trim() == chat)
}

/// Whether to answer a chat that is not allowed (at most once per [`REFUSAL_INTERVAL`]).
fn should_answer_refusal(refusals: &mut HashMap<i64, Instant>, chat: i64, now: Instant) -> bool {
    if refusals.get(&chat).is_some_and(|at| now.duration_since(*at) < REFUSAL_INTERVAL) {
        return false;
    }
    if refusals.len() >= 1000 {
        refusals.retain(|_, at| now.duration_since(*at) < REFUSAL_INTERVAL);
    }
    refusals.insert(chat, now);
    true
}

fn refusal(chat: i64) -> String {
    format!(
        "{} `{}` {}",
        escape("This chat is not allowed. Add"),
        escape_code(&chat.to_string()),
        escape("to the allowed chats in Rustload → Settings → Integrations.")
    )
}

/// The answer to a message from an allowed chat, in MarkdownV2.
async fn respond(engine: &Engine, bot: &Bot, message: &Message) -> String {
    if let Some(document) = &message.document {
        return add_torrent(engine, bot, document).await;
    }
    let text = message.text.as_deref().or(message.caption.as_deref()).unwrap_or_default();
    match command(text).as_deref() {
        Some("status") => status(engine).await,
        Some("queue") => queue(engine).await,
        Some("pause") => set_paused(engine, true).await,
        Some("resume") => set_paused(engine, false).await,
        Some(_) => help(),
        None => add_links(engine, text).await,
    }
}

/// `/status@my_bot more` → `status`.
fn command(text: &str) -> Option<String> {
    let word = text.trim_start().strip_prefix('/')?.split_whitespace().next().unwrap_or_default();
    Some(word.split('@').next().unwrap_or_default().to_ascii_lowercase())
}

fn help() -> String {
    [
        format!("*{}*", escape("Rustload")),
        escape("Send links, magnets or a .torrent file to download them."),
        escape("/status – speed, queue and free disk space"),
        escape("/queue – the first packages of the queue"),
        escape("/pause – pause all downloads"),
        escape("/resume – resume downloads"),
    ]
    .join("\n")
}

async fn add_links(engine: &Engine, text: &str) -> String {
    let parsed = intake::parse_links(text);
    let received = parsed.http.len() + parsed.magnets.len();
    if received == 0 {
        return escape("No link found. Send links, magnets or a .torrent file; /help lists the commands.");
    }
    match intake::add_text(engine, text, &AddOptions::default()).await {
        Ok(ids) => added(engine, &ids, received).await,
        Err(error) => escape(&format!("Could not add the links: {error}")),
    }
}

async fn add_torrent(engine: &Engine, bot: &Bot, document: &Document) -> String {
    let name = document.file_name.clone().unwrap_or_else(|| "telegram.torrent".into());
    let is_torrent = name.to_ascii_lowercase().ends_with(".torrent")
        || document.mime_type.as_deref() == Some("application/x-bittorrent");
    if !is_torrent {
        return escape("Only .torrent files are supported; send links and magnets as text.");
    }
    if document.file_size.is_some_and(|size| size > MAX_TORRENT_SIZE as u64) {
        return escape("This file is too large for a .torrent file.");
    }
    let data = match bot.download(&document.file_id, MAX_TORRENT_SIZE).await {
        Ok(data) => data,
        Err(error) => return escape(&format!("Could not fetch the file: {error}")),
    };
    match intake::add_torrent(engine, &name, data, &AddOptions::default()).await {
        Ok(id) => added(engine, &[id], 1).await,
        Err(error) => escape(&format!("Could not add the torrent: {error}")),
    }
}

/// Describes the packages just added and tells the web interface.
async fn added(engine: &Engine, ids: &[i64], received: usize) -> String {
    let mut lines = Vec::new();
    let mut names = Vec::new();
    for &id in ids {
        let Ok(Some(package)) = db::packages::get(&engine.db, id).await else { continue };
        lines.push(match package.source_kind {
            SourceKind::Links => {
                let files = db::downloads::count_by_package(&engine.db, id).await.unwrap_or(0);
                format!("Added {files} file(s) to “{}”", package.name)
            }
            SourceKind::Magnet | SourceKind::Torrent => {
                format!("Added “{}”: the debrid service downloads it first", package.name)
            }
        });
        names.push(package.name);
    }
    let names = names.join(", ");
    tracing::info!("Telegram bot: received {received} link(s) ({} package(s))", ids.len());
    engine.events.notice(
        Notice::new(NoticeLevel::Info, "telegram_added", format!("Telegram: {names}"))
            .param("name", &names)
            .param("count", received.to_string()),
    );
    escape(&lines.join("\n"))
}

async fn set_paused(engine: &Engine, paused: bool) -> String {
    match engine.set_queue_paused(paused).await {
        Ok(()) => escape(if paused { "Downloads paused." } else { "Downloads resumed." }),
        Err(error) => {
            tracing::warn!("Telegram bot: cannot change the queue state: {error:#}");
            escape("Could not change the queue state.")
        }
    }
}

async fn status(engine: &Engine) -> String {
    match Snapshot::take(engine).await {
        Ok(snapshot) => format_status(&snapshot),
        Err(error) => {
            tracing::warn!("Telegram bot: cannot read the queue: {error:#}");
            escape("Cannot read the queue right now.")
        }
    }
}

fn format_status(snapshot: &Snapshot) -> String {
    let mut lines = vec![format!("*{}*", escape("Rustload"))];
    if snapshot.paused {
        lines.push(escape("Downloads are paused."));
    } else if !snapshot.schedule_open {
        lines.push(escape("Outside the download window of the schedule."));
    }
    lines.push(escape(&format!("Speed: {}/s", human_bytes(snapshot.speed))));
    lines.push(escape(&format!("Active: {} · Queued: {}", snapshot.active, snapshot.queued)));
    let eta = snapshot.eta().map(|secs| format!(", about {}", duration(secs))).unwrap_or_default();
    lines.push(escape(&format!("Remaining: {}{eta}", human_bytes(snapshot.remaining))));
    if let Some(disk) = snapshot.disk {
        lines.push(escape(&format!("Free disk space: {}", human_bytes(disk.free))));
    }
    lines.join("\n")
}

async fn queue(engine: &Engine) -> String {
    let packages = match db::packages::list_queue(&engine.db).await {
        Ok(packages) => packages,
        Err(error) => {
            tracing::warn!("Telegram bot: cannot read the queue: {error:#}");
            return escape("Cannot read the queue right now.");
        }
    };
    if packages.is_empty() {
        return escape("The queue is empty.");
    }
    let shown = &packages[..packages.len().min(QUEUE_LINES)];
    let ids: Vec<i64> = shown.iter().map(|p| p.id).collect();
    let files = db::downloads::list_by_packages(&engine.db, &ids).await.unwrap_or_default();
    let mut lines: Vec<String> = shown
        .iter()
        .enumerate()
        .map(|(index, package)| {
            let package_files = files.get(&package.id).map(Vec::as_slice).unwrap_or_default();
            let (status, progress) = package_state(engine, package, package_files);
            queue_line(index + 1, &package.name, progress, status)
        })
        .collect();
    if packages.len() > QUEUE_LINES {
        lines.push(escape(&format!("… and {} more", packages.len() - QUEUE_LINES)));
    }
    lines.join("\n")
}

/// Status and progress (0..1) of a queued package, live values included.
fn package_state(engine: &Engine, package: &PackageRow, files: &[DownloadRow]) -> (PackageStatus, Option<f64>) {
    let status = engine.package_status(package, files);
    let progress = match package.stage {
        Stage::Remote => engine.remote_progress(package.id).map(|p| p.progress),
        Stage::Verify => engine.live.verification_progress(package.id),
        Stage::Extract => engine.live.extraction_progress(package.id),
        Stage::Download | Stage::Done => {
            let (done, total) = files.iter().fold((0u64, 0u64), |(done, total), file| {
                let downloaded = match engine.live.get(file.id) {
                    Some(live) => live.downloaded.load(Ordering::Relaxed),
                    None => file.downloaded.max(0) as u64,
                };
                (done + downloaded, total + file.size.unwrap_or(0).max(0) as u64)
            });
            (total > 0).then(|| done as f64 / total as f64)
        }
    };
    (status, progress)
}

fn queue_line(position: usize, name: &str, progress: Option<f64>, status: PackageStatus) -> String {
    let progress = progress.map(|p| format!("{:.0}% · ", (p * 100.0).clamp(0.0, 100.0))).unwrap_or_default();
    let status = match status {
        PackageStatus::Queued => "queued",
        PackageStatus::Downloading => "downloading",
        PackageStatus::Paused => "paused",
        PackageStatus::Debrid => "at the debrid service",
        PackageStatus::Verifying => "verifying",
        PackageStatus::Extracting => "extracting",
        PackageStatus::Completed => "completed",
        PackageStatus::Failed => "failed",
    };
    escape(&format!("{position}. {} — {progress}{status}", shorten(name, NAME_MAX_CHARS)))
}

fn shorten(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(max_chars - 1).collect();
    short.push('…');
    short
}

/// `45 s`, `12 min`, `3 h 05 min`, `2 d 4 h`.
fn duration(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs / 60),
        3600..86_400 => format!("{} h {:02} min", secs / 3600, secs % 3600 / 60),
        _ => format!("{} d {} h", secs / 86_400, secs % 86_400 / 3600),
    }
}

/// Characters that MarkdownV2 requires to escape outside of entities.
const MARKDOWN_SPECIAL: &str = r"_*[]()~`>#+-=|{}.!\";

/// Escapes text for MarkdownV2, outside of entities.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        if MARKDOWN_SPECIAL.contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// Escapes text inside a MarkdownV2 `code` entity.
fn escape_code(text: &str) -> String {
    text.replace('\\', r"\\").replace('`', r"\`")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::DiskInfo;

    #[test]
    fn escapes_markdown_v2() {
        assert_eq!(escape("Movie (2024) [1080p] - part_1.mkv!"), r"Movie \(2024\) \[1080p\] \- part\_1\.mkv\!");
        assert_eq!(escape(r"a*b~c`d>e#f+g=h|i{j}k\l"), r"a\*b\~c\`d\>e\#f\+g\=h\|i\{j\}k\\l");
        assert_eq!(escape("Déjà vu → “ok” · 45%"), "Déjà vu → “ok” · 45%");
        assert_eq!(escape_code(r"a`b\c-d.e"), r"a\`b\\c-d.e");
        assert_eq!(
            refusal(-1001234),
            r"This chat is not allowed\. Add `-1001234` to the allowed chats in Rustload → Settings → Integrations\."
        );
    }

    #[test]
    fn parses_commands() {
        assert_eq!(command("/status").as_deref(), Some("status"));
        assert_eq!(command("  /Queue@rustload_bot extra").as_deref(), Some("queue"));
        assert_eq!(command("/").as_deref(), Some(""));
        assert_eq!(command("https://example.com/file.zip"), None);
        assert_eq!(command("see /status"), None);
    }

    #[test]
    fn checks_allowed_chats() {
        let allowed = vec!["42".to_owned(), " -1001234 ".to_owned()];
        assert!(is_allowed(&allowed, 42));
        assert!(is_allowed(&allowed, -1001234));
        assert!(!is_allowed(&allowed, 4));
        assert!(!is_allowed(&[], 42));
    }

    #[test]
    fn answers_refused_chats_once_per_interval() {
        let mut refusals = HashMap::new();
        let start = Instant::now();
        assert!(should_answer_refusal(&mut refusals, 7, start));
        assert!(!should_answer_refusal(&mut refusals, 7, start + Duration::from_secs(60)));
        assert!(should_answer_refusal(&mut refusals, 8, start + Duration::from_secs(60)));
        assert!(should_answer_refusal(&mut refusals, 7, start + REFUSAL_INTERVAL));
    }

    /// A fake Bot API: one batch of updates, then empty polls; records what the bot sends.
    #[derive(Default)]
    struct FakeApi {
        polls: std::sync::Mutex<Vec<Value>>,
        sent: std::sync::Mutex<Vec<Value>>,
    }

    async fn fake_api(
        axum::extract::State(api): axum::extract::State<Arc<FakeApi>>,
        uri: axum::http::Uri,
        axum::Json(params): axum::Json<Value>,
    ) -> axum::Json<Value> {
        let result = match uri.path().rsplit('/').next().unwrap_or_default() {
            "getMe" => json!({ "id": 1, "is_bot": true, "username": "test_bot" }),
            "getUpdates" => {
                let poll = {
                    let mut polls = api.polls.lock().unwrap();
                    polls.push(params);
                    polls.len()
                };
                let message = |id: i64, chat: i64, text: &str| json!({ "update_id": id, "message": { "message_id": id, "chat": { "id": chat }, "text": text } });
                match poll {
                    1 => json!([
                        message(10, 99, "hello"),
                        message(11, 42, "Here: https://example.com/a.zip https://example.com/b.zip"),
                        { "update_id": 12, "edited_message": { "unexpected": true } },
                        message(13, 42, "/help@test_bot"),
                        message(14, 99, "hello again"),
                    ]),
                    // Commands do not wait for the links being added: ask for the queue once
                    // they are.
                    2 => {
                        while api.sent.lock().unwrap().len() < 3 {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        json!([message(15, 42, "/queue")])
                    }
                    _ => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        json!([])
                    }
                }
            }
            "sendMessage" => {
                api.sent.lock().unwrap().push(params);
                json!({ "message_id": 1 })
            }
            _ => return axum::Json(json!({ "ok": false, "error_code": 404, "description": "Not Found" })),
        };
        axum::Json(json!({ "ok": true, "result": result }))
    }

    #[tokio::test]
    async fn answers_messages_through_the_bot_api() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = tempfile::tempdir().unwrap();
        let db = db::open(&dir.path().join("rustload.db")).await.unwrap();
        let settings = Arc::new(crate::settings::SettingsStore::load(db.clone()).await.unwrap());
        settings.modify(|s| s.telegram_bot.allowed_chats = vec!["42".into()]).await.unwrap();
        let config = Arc::new(crate::config::Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            cnl_bind: "127.0.0.1:0".parse().unwrap(),
            config_dir: dir.path().to_path_buf(),
            download_root: dir.path().join("downloads"),
            auth_enabled: true,
            admin_user: None,
            admin_password: None,
            secure_cookies: false,
            log_filter: String::new(),
            puid: None,
            pgid: None,
            umask: None,
        });
        let events = Arc::new(crate::events::EventBus::new());
        let engine = Engine::new(db.clone(), config, settings, events, CancellationToken::new()).await.unwrap();

        let api = Arc::new(FakeApi::default());
        let app = axum::Router::new().fallback(fake_api).with_state(api.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let bot = Bot { http: reqwest::Client::new(), api: format!("http://{address}"), token: "1:abc".into() };
        let stop = CancellationToken::new();
        let task = tokio::spawn(run(engine.clone(), bot, stop.clone()));
        tokio::time::timeout(Duration::from_secs(10), async {
            while api.polls.lock().unwrap().len() < 3 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        stop.cancel();
        task.await.unwrap();

        let offsets: Vec<Value> = api.polls.lock().unwrap().iter().take(3).map(|p| p["offset"].clone()).collect();
        assert_eq!(offsets, [json!(0), json!(15), json!(16)]);
        let mut sent: Vec<(i64, String)> = api
            .sent
            .lock()
            .unwrap()
            .iter()
            .map(|m| (m["chat_id"].as_i64().unwrap(), m["text"].as_str().unwrap().to_owned()))
            .collect();
        // The help may come before or after the links are added; the rest is in order.
        let help = sent.iter().position(|(_, text)| *text == help()).unwrap();
        sent.remove(help);
        assert_eq!(
            sent,
            [(99, refusal(99)), (42, r"Added 2 file\(s\) to “a”".to_owned()), (42, r"1\. a — queued".to_owned()),]
        );
        let queue = db::packages::list_queue(&db).await.unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(db::downloads::count_by_package(&db, queue[0].id).await.unwrap(), 2);
        db.close().await;
    }

    #[test]
    fn formats_status_and_queue() {
        let snapshot = Snapshot {
            speed: 3 * 1024 * 1024,
            active: 2,
            queued: 5,
            remaining: 360 * 3 * 1024 * 1024,
            paused: false,
            schedule_open: true,
            disk: Some(DiskInfo { free: 1536 * 1024 * 1024, total: 0 }),
        };
        assert_eq!(
            format_status(&snapshot),
            "*Rustload*\nSpeed: 3\\.0 MiB/s\nActive: 2 · Queued: 5\nRemaining: 1\\.1 GiB, about 6 min\n\
             Free disk space: 1\\.5 GiB"
        );
        assert_eq!(
            queue_line(1, "My.Show", Some(0.456), PackageStatus::Downloading),
            r"1\. My\.Show — 46% · downloading"
        );
        assert_eq!(queue_line(2, "x", None, PackageStatus::Debrid), r"2\. x — at the debrid service");
        assert_eq!(shorten("abcdef", 4), "abc…");
        assert_eq!(duration(59), "59 s");
        assert_eq!(duration(3 * 3600 + 5 * 60), "3 h 05 min");
        assert_eq!(duration(2 * 86_400 + 4 * 3600), "2 d 4 h");
    }
}
