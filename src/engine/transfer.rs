//! Multi-connection HTTP transfer with dynamic segment splitting and resume support.
//!
//! The first request (`Range: bytes=0-`) both probes the server and starts downloading. When the
//! server honours ranges and the size is known, additional connections *steal* work: each new
//! connection takes the second half of the largest remaining segment (like aria2/IDM). Fast
//! connections therefore end up downloading more, and no connection idles until the very end.
//!
//! Every segment writes at its own offset of a pre-sized temporary file. The remaining ranges are
//! the resume state, persisted periodically by the caller via [`Transfer::checkpoint`].

use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use futures_util::{StreamExt, stream::FuturesUnordered};
use reqwest::{
    StatusCode,
    header::{self, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::{limiter::RateLimiter, live::LiveDownload, naming};
use crate::hosters::Decryptor;

/// Bytes buffered per connection before a positional write. Must stay below the minimum split
/// size so that splitting never cuts into data being written (see [`SegmentSet::acquire`]).
const WRITE_BUFFER: usize = 256 * 1024;
/// Attempts per connection before it gives up on its segment.
const SEGMENT_ATTEMPTS: u32 = 5;

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("stopped")]
    Cancelled,
    /// The server refused the (possibly expired) link: resolving it again may help.
    #[error("access denied by the server (HTTP {0})")]
    Forbidden(u16),
    /// Too many connections (HTTP 429/503): the server limits parallel transfers.
    #[error("the server is busy (HTTP {0})")]
    Busy(u16),
    #[error("{0}")]
    Transient(String),
    #[error("{0}")]
    Fatal(String),
}

impl TransferError {
    fn from_status(status: StatusCode) -> Self {
        match status.as_u16() {
            401 | 403 => Self::Forbidden(status.as_u16()),
            404 | 410 => Self::Fatal(format!("file not found on the server (HTTP {status})")),
            429 | 503 => Self::Busy(status.as_u16()),
            408 | 425 | 500..=599 => Self::Transient(format!("server error (HTTP {status})")),
            _ => Self::Fatal(format!("unexpected server response (HTTP {status})")),
        }
    }

    fn network(error: reqwest::Error) -> Self {
        Self::Transient(format!("network error: {}", root_cause(&error)))
    }

    fn disk(error: io::Error) -> Self {
        Self::Fatal(format!("disk error: {error}"))
    }
}

fn root_cause(error: &(dyn std::error::Error + 'static)) -> String {
    let mut current = error;
    while let Some(source) = current.source() {
        current = source;
    }
    current.to_string()
}

/// What the first response told us about the file.
pub struct Probe {
    pub final_url: String,
    pub size: Option<u64>,
    pub ranges: bool,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub validator: Option<String>,
    /// Body of the probe request, starting at byte 0, reused by the first connection.
    response: Option<reqwest::Response>,
}

pub struct Connection<'a> {
    pub client: &'a reqwest::Client,
    pub headers: &'a HeaderMap,
    pub read_timeout: Duration,
}

impl Connection<'_> {
    /// Sends a GET, waiting at most `read_timeout` for the response headers.
    async fn send(
        &self,
        url: &str,
        range: Option<(u64, Option<u64>)>,
        if_range: Option<&str>,
    ) -> Result<reqwest::Response, TransferError> {
        let mut request = self.client.get(url).headers(self.headers.clone());
        if let Some((start, end)) = range {
            let value = match end {
                Some(end) => format!("bytes={start}-{}", end - 1),
                None => format!("bytes={start}-"),
            };
            request = request.header(header::RANGE, value);
            if let Some(validator) = if_range {
                request = request.header(header::IF_RANGE, validator);
            }
        }
        match tokio::time::timeout(self.read_timeout, request.send()).await {
            Ok(result) => result.map_err(TransferError::network),
            Err(_) => Err(TransferError::Transient("the server did not answer in time".into())),
        }
    }

    async fn get(
        &self,
        url: &str,
        range: Option<(u64, Option<u64>)>,
        if_range: Option<&str>,
    ) -> Result<reqwest::Response, TransferError> {
        let response = self.send(url, range, if_range).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(TransferError::from_status(status));
        }
        Ok(response)
    }

    pub async fn probe(&self, url: &str) -> Result<Probe, TransferError> {
        let response = self.send(url, Some((0, None)), None).await?;
        let response = match response.status() {
            // Empty files cannot satisfy `bytes=0-`: ask for the whole (empty) body instead.
            StatusCode::RANGE_NOT_SATISFIABLE => self.get(url, None, None).await?,
            status if !status.is_success() => return Err(TransferError::from_status(status)),
            _ => response,
        };
        Ok(Probe::from_response(response))
    }
}

impl Probe {
    /// What the headers of a successful response tell about the file.
    pub fn from_response(response: reqwest::Response) -> Self {
        let headers = response.headers();
        let partial = response.status() == StatusCode::PARTIAL_CONTENT;
        let size = if partial {
            headers.get(header::CONTENT_RANGE).and_then(parse_content_range_total)
        } else {
            response.content_length()
        };
        let validator = headers
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .filter(|etag| !etag.starts_with("W/"))
            .or_else(|| headers.get(header::LAST_MODIFIED).and_then(|v| v.to_str().ok()))
            .map(str::to_owned);
        Self {
            final_url: response.url().to_string(),
            size,
            ranges: partial && size.is_some(),
            filename: headers
                .get(header::CONTENT_DISPOSITION)
                .and_then(|v| naming::from_content_disposition(v.as_bytes())),
            content_type: headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_owned),
            validator,
            response: Some(response),
        }
    }

    /// Whether the server answered with a web page.
    pub fn is_html(&self) -> bool {
        self.content_type.as_deref().is_some_and(|ct| ct.to_ascii_lowercase().starts_with("text/html"))
    }
}

/// `bytes 0-99/1234` → 1234.
fn parse_content_range_total(value: &HeaderValue) -> Option<u64> {
    value.to_str().ok()?.rsplit('/').next()?.trim().parse().ok()
}

/// `bytes 100-199/1234` → 100.
fn parse_content_range_start(value: &HeaderValue) -> Option<u64> {
    let value = value.to_str().ok()?.trim();
    let range = value.strip_prefix("bytes")?.trim_start();
    range.split('-').next()?.trim().parse().ok()
}

/// Persisted between runs to resume a partially downloaded file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResumeState {
    pub size: u64,
    pub validator: Option<String>,
    /// Half-open byte ranges still to download.
    pub remaining: Vec<(u64, u64)>,
}

impl ResumeState {
    pub fn downloaded(&self) -> u64 {
        self.size - self.remaining.iter().map(|(start, end)| end - start).sum::<u64>()
    }
}

#[derive(Debug)]
struct SegmentState {
    pos: u64,
    end: u64,
    owned: bool,
}

struct Segment(Mutex<SegmentState>);

impl Segment {
    fn new(pos: u64, end: u64, owned: bool) -> Arc<Self> {
        Arc::new(Self(Mutex::new(SegmentState { pos, end, owned })))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SegmentState> {
        self.0.lock().expect("segment poisoned")
    }

    fn bounds(&self) -> (u64, u64) {
        let state = self.lock();
        (state.pos, state.end)
    }

    fn release(&self) {
        self.lock().owned = false;
    }
}

pub(crate) struct SegmentSet {
    segments: Mutex<Vec<Arc<Segment>>>,
    min_split: u64,
}

impl SegmentSet {
    /// Hands out an unowned unfinished segment, or splits the largest owned one in two.
    ///
    /// Splitting is safe while a writer is mid-flush: the split point is at least `min_split`
    /// past the writer's committed position, and a flush never exceeds `WRITE_BUFFER` bytes.
    fn acquire(&self) -> Option<Arc<Segment>> {
        let mut segments = self.segments.lock().expect("segments poisoned");
        for segment in segments.iter() {
            let mut state = segment.lock();
            if !state.owned && state.pos < state.end {
                state.owned = true;
                return Some(segment.clone());
            }
        }
        let largest = segments
            .iter()
            .filter_map(|segment| {
                let state = segment.lock();
                let remaining = state.end - state.pos;
                (state.owned && remaining >= 2 * self.min_split).then_some((segment.clone(), remaining))
            })
            .max_by_key(|(_, remaining)| *remaining)?
            .0;
        let new = {
            let mut state = largest.lock();
            let remaining = state.end - state.pos;
            if remaining < 2 * self.min_split {
                return None;
            }
            let mid = state.pos + remaining / 2;
            let new = Segment::new(mid, state.end, true);
            state.end = mid;
            new
        };
        segments.push(new.clone());
        Some(new)
    }

    fn remaining(&self) -> Vec<(u64, u64)> {
        self.layout().0
    }

    /// Remaining ranges and the write position of every connection holding a segment, both
    /// sorted. Also read once per second for the segment map.
    pub(crate) fn layout(&self) -> (Vec<(u64, u64)>, Vec<u64>) {
        let segments = self.segments.lock().expect("segments poisoned");
        let mut ranges = Vec::with_capacity(segments.len());
        let mut heads = Vec::new();
        for segment in segments.iter() {
            let state = segment.lock();
            if state.pos < state.end {
                ranges.push((state.pos, state.end));
                if state.owned {
                    heads.push(state.pos);
                }
            }
        }
        ranges.sort_unstable();
        heads.sort_unstable();
        (ranges, heads)
    }
}

pub struct TransferOptions<'a> {
    pub connection: Connection<'a>,
    pub max_connections: u32,
    pub limiter: &'a RateLimiter,
    pub live: &'a LiveDownload,
    /// Applied to the bytes before they are written (encrypted hosters).
    pub decryptor: Option<&'a dyn Decryptor>,
}

/// A transfer in progress, writing into `temp_path`.
pub struct Transfer {
    file: Arc<File>,
    /// Shared with the live registry, which draws the segment map from it.
    segments: Arc<SegmentSet>,
    size: Option<u64>,
    resumable: bool,
    validator: Option<String>,
    url: String,
    first_response: Mutex<Option<reqwest::Response>>,
    /// Connections currently receiving data.
    streaming: AtomicU32,
    /// Lowest number of parallel connections at which the server answered "busy" (`u32::MAX` = never).
    busy_at: AtomicU32,
}

/// Counts a connection as streaming for as long as it lives.
struct StreamingGuard<'a> {
    transfer: &'a Transfer,
    live: &'a LiveDownload,
}

impl<'a> StreamingGuard<'a> {
    fn new(transfer: &'a Transfer, live: &'a LiveDownload) -> Self {
        transfer.streaming.fetch_add(1, Ordering::Relaxed);
        live.connections.fetch_add(1, Ordering::Relaxed);
        Self { transfer, live }
    }
}

impl Drop for StreamingGuard<'_> {
    fn drop(&mut self) {
        self.transfer.streaming.fetch_sub(1, Ordering::Relaxed);
        self.live.connections.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Transfer {
    /// Opens the temporary file, resuming from `resume` when it matches the probe.
    /// Returns the transfer and the number of bytes already present.
    pub fn open(
        mut probe: Probe,
        temp_path: &Path,
        resume: Option<ResumeState>,
        min_split: u64,
    ) -> Result<(Self, u64), TransferError> {
        debug_assert!(min_split > WRITE_BUFFER as u64);
        let existing_len = std::fs::metadata(temp_path).map(|m| m.len()).ok();
        let resume =
            resume.filter(|state| probe.ranges && Some(state.size) == probe.size && existing_len == Some(state.size));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(resume.is_none())
            .open(temp_path)
            .map_err(TransferError::disk)?;

        let (segments, downloaded) = match (&resume, probe.size) {
            (Some(state), _) => {
                let segments = state.remaining.iter().map(|&(start, end)| Segment::new(start, end, false)).collect();
                // The probe body starts at 0, useless when resuming.
                probe.response = None;
                (segments, state.downloaded())
            }
            (None, Some(size)) => {
                if probe.ranges {
                    file.set_len(size).map_err(TransferError::disk)?;
                }
                (vec![Segment::new(0, size, false)], 0)
            }
            // Unknown size: a single open-ended stream.
            (None, None) => (vec![Segment::new(0, u64::MAX, false)], 0),
        };

        Ok((
            Self {
                file: Arc::new(file),
                segments: Arc::new(SegmentSet { segments: Mutex::new(segments), min_split }),
                size: probe.size,
                resumable: probe.ranges,
                validator: probe.validator,
                url: probe.final_url,
                first_response: Mutex::new(probe.response),
                streaming: AtomicU32::new(0),
                busy_at: AtomicU32::new(u32::MAX),
            },
            downloaded,
        ))
    }

    /// Current resume state after flushing written data to disk, `None` if the server does not
    /// support resuming.
    pub async fn checkpoint(&self) -> Option<ResumeState> {
        if !self.resumable {
            return None;
        }
        // Snapshot first: only bytes written before the flush are guaranteed to be on disk.
        let remaining = self.segments.remaining();
        let file = self.file.clone();
        if let Err(error) =
            tokio::task::spawn_blocking(move || file.sync_data()).await.unwrap_or_else(|e| Err(io::Error::other(e)))
        {
            tracing::warn!("cannot flush download to disk: {error}");
            return None;
        }
        Some(ResumeState { size: self.size?, validator: self.validator.clone(), remaining })
    }

    /// Number of parallel connections the server seems to accept, if it refused more.
    pub fn connection_limit(&self) -> Option<u32> {
        Some(self.busy_at.load(Ordering::Relaxed)).filter(|n| *n != u32::MAX)
    }

    fn streaming(&self) -> u32 {
        self.streaming.load(Ordering::Relaxed)
    }

    /// Downloads until every segment is complete. Returns the final size.
    pub async fn run(&self, options: &TransferOptions<'_>, cancel: &CancellationToken) -> Result<u64, TransferError> {
        let stop = cancel.child_token();
        if self.resumable
            && let Some(size) = self.size
        {
            options.live.track_segments(size, self.segments.clone());
        }
        let workers = match (self.resumable, self.size) {
            (true, Some(size)) => {
                let by_size = size.div_ceil(self.segments.min_split).max(1);
                options.max_connections.max(1).min(by_size.min(u32::MAX as u64) as u32)
            }
            _ => 1,
        };
        let mut running: FuturesUnordered<_> = (0..workers).map(|_| self.worker(options, &stop)).collect();

        let mut failure = None;
        while let Some(result) = running.next().await {
            if let Err(error) = result {
                // One connection failing for good stops the others; the task decides what next.
                if failure.is_none() {
                    stop.cancel();
                    failure = Some(error);
                }
            }
        }
        if cancel.is_cancelled() {
            return Err(TransferError::Cancelled);
        }
        if let Some(error) = failure {
            return Err(error);
        }

        let file = self.file.clone();
        tokio::task::spawn_blocking(move || file.sync_all())
            .await
            .unwrap_or_else(|e| Err(io::Error::other(e)))
            .map_err(TransferError::disk)?;

        match self.size {
            Some(size) if !self.segments.remaining().is_empty() => {
                Err(TransferError::Transient(format!("incomplete download ({size} bytes expected)")))
            }
            Some(size) => Ok(size),
            None => Ok(options.live.downloaded.load(Ordering::Relaxed)),
        }
    }

    /// One connection: downloads segments until there is nothing left to take.
    async fn worker(&self, options: &TransferOptions<'_>, stop: &CancellationToken) -> Result<(), TransferError> {
        let first = self.first_response.lock().expect("poisoned").take();
        let mut pending_first = first.and_then(|response| self.segments.acquire().map(|segment| (segment, response)));

        loop {
            let (segment, response) = match pending_first.take() {
                Some((segment, response)) => (segment, Some(response)),
                None => match self.segments.acquire() {
                    Some(segment) => (segment, None),
                    None => return Ok(()),
                },
            };
            match self.fetch_segment(options, stop, &segment, response).await {
                Ok(()) => {}
                // Another connection is making progress: hand the segment over and stop, the
                // server does not want more parallel connections.
                Err(TransferError::Busy(_) | TransferError::Forbidden(_) | TransferError::Transient(_))
                    if self.streaming() > 0 =>
                {
                    segment.release();
                    return Ok(());
                }
                Err(error) => {
                    segment.release();
                    return Err(error);
                }
            }
        }
    }

    /// Downloads one segment, reconnecting on network errors.
    async fn fetch_segment(
        &self,
        options: &TransferOptions<'_>,
        stop: &CancellationToken,
        segment: &Segment,
        mut response: Option<reqwest::Response>,
    ) -> Result<(), TransferError> {
        let mut attempt = 0;
        let mut progress_mark = segment.bounds().0;
        loop {
            let (pos, end) = segment.bounds();
            if pos >= end {
                return Ok(());
            }
            let opened = match response.take() {
                Some(response) => Ok(response),
                None => tokio::select! {
                    _ = stop.cancelled() => return Err(TransferError::Cancelled),
                    opened = self.open_range(options, pos, end) => opened,
                },
            };
            let outcome = match opened {
                Ok(response) => self.stream_into(options, stop, segment, response).await,
                Err(TransferError::Busy(status)) if self.streaming() > 0 => {
                    // Remember how many parallel connections the server tolerates.
                    self.busy_at.fetch_min(self.streaming(), Ordering::Relaxed);
                    return Err(TransferError::Busy(status));
                }
                Err(error) => Err(error),
            };
            let (pos, end) = segment.bounds();
            match outcome {
                Ok(()) if pos >= end => return Ok(()),
                // Unknown size: the end of the stream is the end of the file.
                Ok(()) if self.size.is_none() => {
                    let mut state = segment.lock();
                    state.end = state.pos;
                    return Ok(());
                }
                Ok(()) => tracing::debug!("connection closed early at byte {pos}"),
                Err(error @ (TransferError::Transient(_) | TransferError::Busy(_))) if self.resumable => {
                    tracing::debug!("segment retry {}: {error}", attempt + 1);
                }
                Err(error) => return Err(error),
            }
            if !self.resumable {
                return Err(TransferError::Transient(
                    "connection lost and the server does not support resuming".into(),
                ));
            }
            // Only consecutive failures without progress count against the segment.
            if pos > progress_mark {
                progress_mark = pos;
                attempt = 0;
            }
            attempt += 1;
            if attempt >= SEGMENT_ATTEMPTS {
                return Err(TransferError::Transient("connection keeps dropping".into()));
            }
            // Exponential backoff with jitter, so connections refused together do not retry together.
            let delay = Duration::from_millis((1000 << attempt) + rand::random::<u64>() % 1000);
            tokio::select! {
                _ = stop.cancelled() => return Err(TransferError::Cancelled),
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }

    async fn open_range(
        &self,
        options: &TransferOptions<'_>,
        pos: u64,
        end: u64,
    ) -> Result<reqwest::Response, TransferError> {
        // With If-Range, a server whose file changed answers 200 with the new file.
        let response = options.connection.get(&self.url, Some((pos, Some(end))), self.validator.as_deref()).await?;
        let content_range = response.headers().get(header::CONTENT_RANGE);
        let start = content_range.and_then(parse_content_range_start);
        let total = content_range.and_then(parse_content_range_total);
        if response.status() == StatusCode::OK && self.validator.is_some() {
            // If-Range mismatch: the file changed, or a CDN node answered with another validator.
            return Err(TransferError::Transient(
                "the server sent the whole file instead of the requested part".into(),
            ));
        }
        if response.status() != StatusCode::PARTIAL_CONTENT || start != Some(pos) {
            return Err(TransferError::Transient("the server stopped honouring range requests".into()));
        }
        if total.is_some() && total != self.size {
            return Err(TransferError::Fatal("the file changed on the server during the download".into()));
        }
        Ok(response)
    }

    /// Streams a response body into the segment's region of the file.
    async fn stream_into(
        &self,
        options: &TransferOptions<'_>,
        stop: &CancellationToken,
        segment: &Segment,
        response: reqwest::Response,
    ) -> Result<(), TransferError> {
        let _streaming = StreamingGuard::new(self, options.live);
        let mut stream = response.bytes_stream();
        let mut buffer = Vec::with_capacity(WRITE_BUFFER);
        options.live.set_downloading();
        loop {
            let next = tokio::select! {
                biased;
                _ = stop.cancelled() => {
                    self.flush(options, segment, &mut buffer).await?;
                    return Err(TransferError::Cancelled);
                }
                next = tokio::time::timeout(options.connection.read_timeout, stream.next()) => next,
            };
            match next {
                Err(_) => {
                    self.flush(options, segment, &mut buffer).await?;
                    return Err(TransferError::Transient("read timeout".into()));
                }
                Ok(None) => {
                    self.flush(options, segment, &mut buffer).await?;
                    return Ok(());
                }
                Ok(Some(Err(error))) => {
                    self.flush(options, segment, &mut buffer).await?;
                    return Err(TransferError::network(error));
                }
                Ok(Some(Ok(chunk))) => {
                    options.limiter.consume(chunk.len()).await;
                    buffer.extend_from_slice(&chunk);
                    if buffer.len() >= WRITE_BUFFER && self.flush(options, segment, &mut buffer).await? {
                        // Segment complete (possibly shortened by a split): drop the connection.
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Writes the buffer at the segment position. Returns whether the segment is complete.
    async fn flush(
        &self,
        options: &TransferOptions<'_>,
        segment: &Segment,
        buffer: &mut Vec<u8>,
    ) -> Result<bool, TransferError> {
        let (offset, len) = {
            let state = segment.lock();
            (state.pos, (buffer.len() as u64).min(state.end.saturating_sub(state.pos)) as usize)
        };
        if len > 0 {
            if let Some(decryptor) = options.decryptor {
                decryptor.apply(offset, &mut buffer[..len]);
            }
            let file = self.file.clone();
            let data = std::mem::take(buffer);
            let data = tokio::task::spawn_blocking(move || write_all_at(&file, &data[..len], offset).map(|()| data))
                .await
                .unwrap_or_else(|e| Err(io::Error::other(e)))
                .map_err(TransferError::disk)?;
            *buffer = data;
            segment.lock().pos += len as u64;
            options.live.downloaded.fetch_add(len as u64, Ordering::Relaxed);
        }
        buffer.clear();
        let (pos, end) = segment.bounds();
        Ok(pos >= end)
    }
}

#[cfg(unix)]
fn write_all_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
fn write_all_at(file: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let written = file.seek_write(buf, offset)?;
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        buf = &buf[written..];
        offset += written as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ranges: &[(u64, u64, bool)], min_split: u64) -> SegmentSet {
        SegmentSet { segments: Mutex::new(ranges.iter().map(|&(p, e, o)| Segment::new(p, e, o)).collect()), min_split }
    }

    #[test]
    fn hands_out_unowned_segments_first() {
        let segments = set(&[(0, 100, true), (100, 200, false)], 10);
        let segment = segments.acquire().unwrap();
        assert_eq!(segment.bounds(), (100, 200));
    }

    #[test]
    fn splits_largest_segment_in_half() {
        let segments = set(&[(0, 100, true), (100, 400, true)], 10);
        let segment = segments.acquire().unwrap();
        assert_eq!(segment.bounds(), (250, 400));
        assert_eq!(segments.remaining(), vec![(0, 100), (100, 250), (250, 400)]);
    }

    #[test]
    fn refuses_to_split_small_segments() {
        let segments = set(&[(0, 15, true)], 10);
        assert!(segments.acquire().is_none());
    }

    #[test]
    fn reports_remaining_ranges_and_connection_heads() {
        let segments = set(&[(300, 400, false), (0, 100, true), (100, 100, true), (150, 300, true)], 10);
        assert_eq!(segments.layout(), (vec![(0, 100), (150, 300), (300, 400)], vec![0, 150]));
    }

    #[test]
    fn resume_state_counts_downloaded_bytes() {
        let state = ResumeState { size: 1000, validator: None, remaining: vec![(100, 200), (900, 1000)] };
        assert_eq!(state.downloaded(), 800);
    }

    #[test]
    fn parses_content_range() {
        let value = HeaderValue::from_static("bytes 100-199/1234");
        assert_eq!(parse_content_range_total(&value), Some(1234));
        assert_eq!(parse_content_range_start(&value), Some(100));
        assert_eq!(parse_content_range_total(&HeaderValue::from_static("bytes 0-1/*")), None);
    }
}
