//! Link checks against a local HTTP server and a scratch database.

use std::{net::SocketAddr, path::Path, sync::Arc};

use axum::{
    Router,
    http::{StatusCode, header},
    response::Html,
    routing::get,
};
use tokio_util::sync::CancellationToken;

use super::*;
use crate::{config::Config, engine::intake, events::EventBus, hosters::FolderEntry, settings::SettingsStore};

const HASH: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";

async fn engine(dir: &Path) -> Arc<Engine> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let db = db::open(&dir.join("rustload.db")).await.unwrap();
    let settings = Arc::new(SettingsStore::load(db.clone()).await.unwrap());
    let config = Arc::new(Config::from_env().unwrap());
    Engine::new(db, config, settings, Arc::new(EventBus::new()), CancellationToken::new()).await.unwrap()
}

/// A file, a dead link and a page listing links.
async fn server() -> SocketAddr {
    let page = r#"<html><head><base href="/files/"><link href="/style.css" rel="stylesheet"></head><body>
        <a href="Show.part1.rar">1</a> <a href="Show.part2.rar">2</a> <a href="about.html">about</a>
        <a href="https://mega.nz/file/AbC#k3y">mirror</a> <a href="magnet:?xt=urn:btih:abc&amp;dn=Show">magnet</a>
        </body></html>"#;
    let app = Router::new()
        .route(
            "/file.bin",
            get(|| async {
                ([(header::CONTENT_DISPOSITION, r#"attachment; filename="report.pdf""#)], vec![7u8; 1234])
            }),
        )
        .route("/gone", get(|| async { StatusCode::NOT_FOUND }))
        .route("/page", get(move || async move { Html(page) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// HLS streams and M3U lists, with and without their usual extension.
async fn playlist_server() -> SocketAddr {
    const HLS: &str = "application/vnd.apple.mpegurl";
    let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1280000,RESOLUTION=640x360\nlow/index.m3u8\n";
    let media = "#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:9.9,\nsegment0.ts\n#EXT-X-ENDLIST\n";
    let list = "#EXTM3U\n#EXTINF:215,Artist - First song\nsongs/first.mp3\nsongs/second.mp3\n\
                https://mega.nz/file/AbC#k3y\n";
    let app = Router::new()
        .route("/films/big-buck-bunny/master.m3u8", get(move || async move { ([(header::CONTENT_TYPE, HLS)], master) }))
        .route("/live", get(move || async move { ([(header::CONTENT_TYPE, "application/x-mpegURL")], media) }))
        .route("/lists/road-trip.m3u", get(move || async move { ([(header::CONTENT_TYPE, "audio/x-mpegurl")], list) }))
        .route("/radio", get(move || async move { ([(header::CONTENT_TYPE, "audio/x-mpegurl")], list) }))
        .route("/empty.m3u", get(|| async { "#EXTM3U\n" }))
        .route("/private/master.m3u8", get(|| async { StatusCode::FORBIDDEN }))
        .route("/gone.m3u", get(|| async { StatusCode::NOT_FOUND }))
        .route("/login.m3u8", get(|| async { Html("<html><body>Sign in</body></html>") }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

fn links(urls: &[String]) -> Vec<Link> {
    urls.iter().map(|url| Link::parse(url).unwrap()).collect()
}

#[tokio::test]
async fn checks_files_pages_and_magnets() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path()).await;
    let base = format!("http://{}", server().await);
    let file = format!("{base}/file.bin");
    let queued = intake::add_links(&engine, std::slice::from_ref(&file), &Default::default()).await.unwrap();
    let finished = sqlx::query_scalar::<_, i64>(
        "INSERT INTO packages (name, create_subfolder, extract, delete_archives, source_kind, source, stage,
            position, created_at, finished_at)
         VALUES ('Show', 1, 0, 0, 'magnet', ?, 'done', 1, 0, 1) RETURNING id",
    )
    .bind("magnet:?xt=urn:btih:YEX6DQDLXISUVHOJ6UM3GNNKPQJWPKEK&dn=Show")
    .fetch_one(&engine.db)
    .await
    .unwrap();

    let urls = [file, format!("{base}/gone"), format!("{base}/page"), format!("magnet:?xt=urn:btih:{HASH}&dn=Show")];
    let results = check_links(&engine, links(&urls), true).await.unwrap();
    assert_eq!(results.iter().map(|r| r.url.as_str()).collect::<Vec<_>>(), urls);

    let file = &results[0];
    assert_eq!((file.status, file.handler.kind), (LinkStatus::Online, HandlerKind::Direct));
    assert_eq!(
        (file.filename.as_deref(), file.size, file.host.as_deref()),
        (Some("report.pdf"), Some(1234), Some("127.0.0.1"))
    );
    let duplicate = file.duplicate.as_ref().unwrap();
    assert_eq!((duplicate.location, duplicate.package_id), (DuplicateLocation::Queue, queued));

    let gone = &results[1];
    assert_eq!(gone.status, LinkStatus::Offline);
    assert!(gone.error.as_deref().unwrap().contains("404"), "{:?}", gone.error);
    assert!(gone.duplicate.is_none());

    let page = &results[2];
    assert_eq!((page.status, page.handler.kind), (LinkStatus::Unsupported, HandlerKind::Page));
    assert_eq!(
        page.found.as_deref().unwrap(),
        [
            format!("{base}/files/Show.part1.rar"),
            format!("{base}/files/Show.part2.rar"),
            "https://mega.nz/file/AbC#k3y".into(),
            "magnet:?xt=urn:btih:abc&dn=Show".into(),
        ]
    );

    let magnet = &results[3];
    assert_eq!(
        (magnet.kind, magnet.status, magnet.handler.kind),
        (LinkKind::Magnet, LinkStatus::Unsupported, HandlerKind::Magnet)
    );
    assert_eq!(magnet.magnet.as_ref().map(|m| m.info_hash.as_str()), Some(HASH));
    assert_eq!(magnet.filename.as_deref(), Some("Show"));
    let duplicate = magnet.duplicate.as_ref().unwrap();
    assert_eq!((duplicate.location, duplicate.package_id), (DuplicateLocation::History, finished));
}

#[tokio::test]
async fn pages_are_scanned_only_on_request() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path()).await;
    let results = check_links(&engine, links(&[format!("http://{}/page", server().await)]), false).await.unwrap();
    assert_eq!(results[0].handler.kind, HandlerKind::Page);
    assert!(results[0].found.is_none());
}

#[tokio::test]
async fn selects_handlers_in_resolution_order() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path()).await;
    // No debrid account: plugins, then the link itself.
    let mega = HandlerInfo::of(&Handler::select(&engine, "https://mega.nz/file/AbC#k3y").await);
    assert_eq!((mega.kind, mega.id.as_deref(), mega.name.as_str()), (HandlerKind::Hoster, Some("mega"), "MEGA"));
    let direct = HandlerInfo::of(&Handler::select(&engine, "https://example.com/a.zip").await);
    assert_eq!((direct.kind, direct.id, direct.account_id), (HandlerKind::Direct, None, None));
}

#[tokio::test]
async fn checks_playlists() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path()).await;
    // Streams are reported without running yt-dlp.
    engine.settings.modify(|s| s.media.enabled = false).await.unwrap();
    let base = format!("http://{}", playlist_server().await);
    let urls = [
        "/films/big-buck-bunny/master.m3u8",
        "/live",
        "/private/master.m3u8",
        "/lists/road-trip.m3u",
        "/empty.m3u",
        "/login.m3u8",
        "/gone.m3u",
    ]
    .map(|path| format!("{base}{path}"));
    let results = check_links(&engine, links(&urls), false).await.unwrap();

    // The private one is left to yt-dlp, which can send the headers its site expects.
    for stream in &results[..3] {
        assert_eq!(
            (stream.handler.kind, stream.handler.id.as_deref(), stream.handler.name.as_str()),
            (HandlerKind::Media, Some("hls"), "HLS")
        );
        assert_eq!((stream.status, stream.error.as_deref()), (LinkStatus::Unsupported, Some(m3u::NEEDS_YTDLP)));
    }

    let list = &results[3];
    assert_eq!((list.status, list.filename.as_deref(), list.size), (LinkStatus::Online, None, None));
    let json = serde_json::to_value(list).unwrap();
    assert_eq!(
        json["handler"],
        serde_json::json!({ "type": "playlist", "id": "m3u", "name": "M3U", "account_id": null })
    );
    let folder = list.folder.as_ref().unwrap();
    assert_eq!((folder.name.as_deref(), folder.files, folder.size), (Some("road-trip"), 3, None));
    let names: Vec<&str> = folder.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Artist - First song", "second.mp3", "AbC"]);

    assert_eq!((results[4].handler.kind, results[4].status), (HandlerKind::Playlist, LinkStatus::Offline));
    // Not a playlist after all, or a list that cannot be read: the usual route.
    assert_eq!((results[5].handler.kind, results[5].status), (HandlerKind::Page, LinkStatus::Unsupported));
    assert_eq!((results[6].handler.kind, results[6].status), (HandlerKind::Direct, LinkStatus::Offline));
}

#[tokio::test]
async fn adds_playlists_as_streams_and_links() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path()).await;
    let base = format!("http://{}", playlist_server().await);
    let urls = [format!("{base}/lists/road-trip.m3u"), format!("{base}/films/big-buck-bunny/master.m3u8")];
    let id = intake::add_links(&engine, &urls, &Default::default()).await.unwrap();

    let package = db::packages::get(&engine.db, id).await.unwrap().unwrap();
    assert_eq!((package.name.as_str(), package.name_locked), ("road-trip", true));
    // The stream goes to yt-dlp with the package's media options.
    assert!(package.media_options().is_some());
    let files = db::downloads::list_by_package(&engine.db, id).await.unwrap();
    let urls: Vec<String> = files.iter().map(|d| d.url.clone()).collect();
    assert_eq!(
        urls,
        [
            format!("{base}/lists/songs/first.mp3"),
            format!("{base}/lists/songs/second.mp3"),
            "https://mega.nz/file/AbC#k3y".into(),
            format!("{base}/films/big-buck-bunny/master.m3u8"),
        ]
    );
    assert!(files[..3].iter().all(|d| d.media.is_none()));
    assert_eq!(files[3].media_meta().and_then(|m| m.title).as_deref(), Some("big-buck-bunny"));

    // A list recognised by its type only is replaced by its links when the availability check
    // reads it.
    let id = intake::add_links(&engine, &[format!("{base}/radio")], &Default::default()).await.unwrap();
    let added = db::downloads::list_by_package(&engine.db, id).await.unwrap();
    assert_eq!(added.len(), 1);
    assert!(crate::engine::m3u::settle_queued(&engine, &added[0]).await);
    let files = db::downloads::list_by_package(&engine.db, id).await.unwrap();
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].url, format!("{base}/songs/first.mp3"));
}

#[test]
fn accepts_http_and_magnet_links_only() {
    let link = Link::parse("  https://mega.nz/file/AbC#key ").unwrap();
    assert_eq!((link.kind, link.url.as_str()), (LinkKind::Http, "https://mega.nz/file/AbC#key"));
    assert_eq!(Link::parse("magnet:?xt=urn:btih:abc").unwrap().kind, LinkKind::Magnet);
    for invalid in ["ftp://host/file", "file.zip", "", "javascript:alert(1)"] {
        assert!(Link::parse(invalid).is_none(), "{invalid}");
    }
}

#[test]
fn previews_folders() {
    let entry = |url: &str, name: Option<&str>, size: Option<u64>| FolderEntry {
        url: url.into(),
        name: name.map(str::to_owned),
        size,
        rel_dir: String::new(),
    };
    let listing = FolderListing {
        name: Some("Album".into()),
        entries: vec![entry("https://h.com/1", Some("a.flac"), Some(10)), entry("https://h.com/b.flac", None, Some(5))],
    };
    let preview = FolderPreview::from(listing.clone());
    assert_eq!((preview.files, preview.size), (2, Some(15)));
    assert_eq!(preview.entries[1].name, "b.flac");

    let mut unknown = listing;
    unknown.entries.extend((0..250).map(|i| entry(&format!("https://h.com/{i}"), None, None)));
    let preview = FolderPreview::from(unknown);
    assert_eq!((preview.files, preview.size, preview.entries.len()), (252, None, MAX_FOLDER_ENTRIES));
}

#[test]
fn serializes_the_documented_shape() {
    let link = Link::parse("https://example.com/file.zip").unwrap();
    let json = serde_json::to_value(LinkInfo::new(&link, HandlerInfo::plain(HandlerKind::Direct, "Direct"))).unwrap();
    assert_eq!((json["kind"].as_str(), json["status"].as_str()), (Some("http"), Some("unknown")));
    assert_eq!(
        json["handler"],
        serde_json::json!({ "type": "direct", "id": null, "name": "Direct", "account_id": null })
    );
    for key in ["host", "filename", "size", "error", "duplicate", "folder", "found", "magnet"] {
        assert!(json[key].is_null(), "{key}");
    }
}
