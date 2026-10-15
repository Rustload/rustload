//! End-to-end tests of the segmented transfer against a local HTTP server.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use reqwest::header::HeaderMap;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;

use super::{
    limiter::RateLimiter,
    live::{LiveRegistry, segment_map},
    transfer::{Connection, Transfer, TransferOptions},
};

const SIZE: usize = 8 * 1024 * 1024;
const MIN_SPLIT: u64 = 1024 * 1024;

/// Throttling applied by the test server: at most `limit` simultaneous requests (429 above),
/// each body chunk of 64 KiB delayed by `delay`.
#[derive(Clone, Copy)]
struct Throttle {
    limit: u32,
    delay: Duration,
}

/// Serves `dir` with Range support, plus `/plain/data.bin` that ignores ranges.
async fn start_server(dir: &Path, throttle: Option<Throttle>) -> SocketAddr {
    let data = Arc::new(std::fs::read(dir.join("data.bin")).unwrap());
    let plain = {
        let data = data.clone();
        get(move || {
            let data = data.clone();
            async move { Body::from(data.to_vec()) }
        })
    };
    let mut app = Router::new().route("/plain/data.bin", plain).fallback_service(ServeDir::new(dir));
    if let Some(throttle) = throttle {
        let active = Arc::new(AtomicU32::new(0));
        app = app.layer(middleware::from_fn_with_state((active, throttle), limit_connections));
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// Answers 429 above the limit of simultaneous requests and streams bodies slowly.
async fn limit_connections(
    State((active, throttle)): State<(Arc<AtomicU32>, Throttle)>,
    request: Request,
    next: Next,
) -> Response {
    if active.fetch_add(1, Ordering::SeqCst) >= throttle.limit {
        active.fetch_sub(1, Ordering::SeqCst);
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let response = next.run(request).await;
    // Hold the slot until the body is fully sent.
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    Response::from_parts(parts, slow_body(bytes, active.clone(), throttle.delay))
}

/// Releases a concurrency slot when dropped, including when the client hangs up early.
struct Slot(Arc<AtomicU32>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Streams `bytes` slowly enough for connections to overlap, holding the slot meanwhile.
fn slow_body(bytes: bytes::Bytes, active: Arc<AtomicU32>, delay: Duration) -> Body {
    let chunks: Vec<bytes::Bytes> = bytes.chunks(64 * 1024).map(bytes::Bytes::copy_from_slice).collect();
    let stream = futures_util::stream::unfold((chunks.into_iter(), Slot(active)), move |(mut iter, slot)| async move {
        let chunk = iter.next()?;
        tokio::time::sleep(delay).await;
        Some((Ok::<_, std::io::Error>(chunk), (iter, slot)))
    });
    Body::from_stream(stream)
}

fn fixture() -> (tempfile::TempDir, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let data: Vec<u8> = (0..SIZE).map(|i| (i * 31 % 251) as u8).collect();
    std::fs::write(dir.path().join("data.bin"), &data).unwrap();
    (dir, data)
}

struct Harness {
    client: reqwest::Client,
    headers: HeaderMap,
    limiter: RateLimiter,
    registry: LiveRegistry,
}

impl Harness {
    fn new() -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Self {
            client: reqwest::Client::builder().http1_only().build().unwrap(),
            headers: HeaderMap::new(),
            limiter: RateLimiter::new(None),
            registry: LiveRegistry::default(),
        }
    }

    fn connection(&self) -> Connection<'_> {
        Connection { client: &self.client, headers: &self.headers, read_timeout: Duration::from_secs(10) }
    }

    async fn download(&self, url: &str, temp: &Path, connections: u32) -> (Transfer, u64) {
        let probe = self.connection().probe(url).await.unwrap();
        let (transfer, _) = Transfer::open(probe, temp, None, MIN_SPLIT).unwrap();
        let live = self.registry.register(1, 1);
        let options = TransferOptions {
            connection: self.connection(),
            max_connections: connections,
            limiter: &self.limiter,
            live: &live,
            decryptor: None,
        };
        let size = transfer.run(&options, &CancellationToken::new()).await.unwrap();
        (transfer, size)
    }
}

fn temp_path(dir: &Path) -> PathBuf {
    dir.join("out.part")
}

#[tokio::test]
async fn downloads_with_several_connections() {
    let (dir, data) = fixture();
    let addr = start_server(dir.path(), None).await;
    let out = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    let (_, size) = harness.download(&format!("http://{addr}/data.bin"), &temp_path(out.path()), 8).await;
    assert_eq!(size, SIZE as u64);
    assert_eq!(std::fs::read(temp_path(out.path())).unwrap(), data);
    assert_eq!(harness.registry.get(1).unwrap().segment_view(), (Some("9".repeat(100)), Vec::new()));
}

#[tokio::test]
async fn falls_back_to_one_connection_without_ranges() {
    let (dir, data) = fixture();
    let addr = start_server(dir.path(), None).await;
    let out = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    let probe = harness.connection().probe(&format!("http://{addr}/plain/data.bin")).await.unwrap();
    assert!(!probe.ranges);
    let (_, size) = harness.download(&format!("http://{addr}/plain/data.bin"), &temp_path(out.path()), 8).await;
    assert_eq!(size, SIZE as u64);
    assert_eq!(std::fs::read(temp_path(out.path())).unwrap(), data);
    assert_eq!(harness.registry.get(1).unwrap().segment_view(), (None, Vec::new()));
}

#[tokio::test]
async fn recognises_web_pages_whatever_the_case() {
    let page = get(|| async { ([(header::CONTENT_TYPE, "Text/HTML; charset=UTF-8")], "<html></html>") });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, Router::new().route("/page", page)).await.unwrap() });
    let harness = Harness::new();
    let probe = harness.connection().probe(&format!("http://{addr}/page")).await.unwrap();
    assert!(probe.is_html());
}

#[tokio::test]
async fn adapts_to_connection_limits() {
    let (dir, data) = fixture();
    let addr = start_server(dir.path(), Some(Throttle { limit: 2, delay: Duration::from_millis(2) })).await;
    let out = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    let (transfer, size) = harness.download(&format!("http://{addr}/data.bin"), &temp_path(out.path()), 8).await;
    assert_eq!(size, SIZE as u64);
    assert_eq!(std::fs::read(temp_path(out.path())).unwrap(), data);
    let limit = transfer.connection_limit().expect("the limit should be detected");
    assert!(limit <= 2, "detected limit {limit}");
}

#[tokio::test]
async fn resumes_an_interrupted_download() {
    let (dir, data) = fixture();
    let addr = start_server(dir.path(), Some(Throttle { limit: 64, delay: Duration::from_millis(20) })).await;
    let out = tempfile::tempdir().unwrap();
    let temp = temp_path(out.path());
    let harness = Harness::new();
    let url = format!("http://{addr}/data.bin");

    // First run: stop after a short while and keep the resume state.
    let probe = harness.connection().probe(&url).await.unwrap();
    let (transfer, _) = Transfer::open(probe, &temp, None, MIN_SPLIT).unwrap();
    let live = harness.registry.register(1, 1);
    let options = TransferOptions {
        connection: harness.connection(),
        max_connections: 4,
        limiter: &harness.limiter,
        live: &live,
        decryptor: None,
    };
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        stopper.cancel();
    });
    assert!(transfer.run(&options, &cancel).await.is_err());
    let state = transfer.checkpoint().await.expect("resumable");
    assert!(state.downloaded() > 0 && state.downloaded() < SIZE as u64, "downloaded {}", state.downloaded());
    let (map, heads) = live.segment_view();
    assert_eq!(map, Some(segment_map(SIZE as u64, &state.remaining)));
    assert!(heads.is_empty(), "stopped connections keep no segment: {heads:?}");
    drop(transfer);

    // Second run picks up where the first stopped.
    let probe = harness.connection().probe(&url).await.unwrap();
    let (transfer, already) = Transfer::open(probe, &temp, Some(state.clone()), MIN_SPLIT).unwrap();
    assert_eq!(already, state.downloaded());
    let live = harness.registry.register(2, 1);
    let options = TransferOptions {
        connection: harness.connection(),
        max_connections: 4,
        limiter: &harness.limiter,
        live: &live,
        decryptor: None,
    };
    transfer.run(&options, &CancellationToken::new()).await.unwrap();
    assert_eq!(live.downloaded.load(Ordering::Relaxed), SIZE as u64 - already);
    assert_eq!(std::fs::read(&temp).unwrap(), data);
}
