//! Link grabber: checks links before they are added — who will download them, whether they are
//! online, their name and size, folder contents, duplicates — and scans web pages for links.
//! Nothing is added here: the interface adds the links the user keeps through the usual intake.

mod page;

use std::{collections::HashMap, time::Duration};

use futures_util::{StreamExt, stream};
use serde::Serialize;
use tokio::time::{Instant, timeout_at};

use super::{
    Engine,
    checker::{self, Availability},
    handler::Handler,
    intake::IntakeError,
    m3u::{self, Playlist},
    magnet, naming,
};
use crate::{
    db::{self, packages::KnownLink},
    debrid::{self, AccountHandle},
    hosters::{FolderListing, HosterError},
    media::{self, MediaError, MediaInfo, sites::Site},
    util::url_host,
};

/// Links checked at the same time.
const CONCURRENCY: usize = 8;
/// Time budget of one link (handler, folder listing or probe, page scan).
const LINK_TIMEOUT: Duration = Duration::from_secs(20);
/// Time budget of a whole request: links not checked by then come back `unknown`.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Folder entries included in a preview.
const MAX_FOLDER_ENTRIES: usize = 200;
/// Bytes of a page read when scanning it for links.
const MAX_PAGE_BYTES: usize = 5 * 1024 * 1024;
/// Links returned for one scanned page.
const MAX_FOUND: usize = 1000;
const TIMED_OUT: &str = "the check took too long";
/// yt-dlp's own time limit ends this long before the link's deadline, so that it stops the probe
/// (and its process) itself rather than being dropped.
const MEDIA_MARGIN: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    Http,
    Magnet,
}

/// A link to check.
#[derive(Debug)]
pub struct Link {
    pub kind: LinkKind,
    pub url: String,
}

impl Link {
    /// An http(s) URL or a magnet URI; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        let url = text.trim();
        let kind = if magnet::is_magnet(url) {
            LinkKind::Magnet
        } else {
            let parsed = url::Url::parse(url).ok()?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return None;
            }
            LinkKind::Http
        };
        Some(Self { kind, url: url.to_owned() })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HandlerKind {
    Debrid,
    Hoster,
    Direct,
    /// A web page: nothing downloads it, but it can be scanned for links.
    Page,
    Magnet,
    /// A media site downloaded with yt-dlp.
    Media,
    /// An M3U list of links: added as its entries, like a folder.
    Playlist,
}

/// Who will download a link, as shown to users.
#[derive(Debug, Serialize)]
pub struct HandlerInfo {
    #[serde(rename = "type")]
    pub kind: HandlerKind,
    /// Debrid provider id or hoster plugin id.
    pub id: Option<String>,
    pub name: String,
    /// Debrid account used first.
    pub account_id: Option<i64>,
}

impl HandlerInfo {
    fn of(handler: &Handler<'_>) -> Self {
        match handler {
            Handler::Debrid(account) => Self::account(HandlerKind::Debrid, account),
            Handler::Hoster(hoster) => Self {
                kind: HandlerKind::Hoster,
                id: Some(hoster.name().to_owned()),
                name: hoster.label().to_owned(),
                account_id: None,
            },
            Handler::Direct => Self::plain(HandlerKind::Direct, "Direct"),
        }
    }

    fn media(site: Site) -> Self {
        Self { kind: HandlerKind::Media, id: Some(site.id.to_owned()), name: site.name.to_owned(), account_id: None }
    }

    fn account(kind: HandlerKind, account: &AccountHandle) -> Self {
        let name = debrid::provider(account.provider).map_or(account.provider, |provider| provider.name);
        Self { kind, id: Some(account.provider.to_owned()), name: name.to_owned(), account_id: Some(account.id) }
    }

    fn plain(kind: HandlerKind, name: &str) -> Self {
        Self { kind, id: None, name: name.to_owned(), account_id: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkStatus {
    Online,
    Offline,
    Unknown,
    /// Nothing can download the link as it is (magnet without account, web page).
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DuplicateLocation {
    Queue,
    History,
}

/// The package already holding a link.
#[derive(Debug, Clone, Serialize)]
pub struct Duplicate {
    pub location: DuplicateLocation,
    pub package_id: i64,
    pub package_name: String,
}

impl From<KnownLink> for Duplicate {
    fn from(known: KnownLink) -> Self {
        Self {
            location: if known.finished { DuplicateLocation::History } else { DuplicateLocation::Queue },
            package_id: known.package_id,
            package_name: known.package_name,
        }
    }
}

/// Content of a folder link; the folder link itself is added, intake expands it.
#[derive(Debug, Serialize)]
pub struct FolderPreview {
    pub name: Option<String>,
    pub files: usize,
    /// Total size, `None` unless every file size is known.
    pub size: Option<u64>,
    /// The first [`MAX_FOLDER_ENTRIES`] files.
    pub entries: Vec<FolderPreviewEntry>,
}

#[derive(Debug, Serialize)]
pub struct FolderPreviewEntry {
    pub name: String,
    pub size: Option<u64>,
    pub rel_dir: String,
}

impl From<FolderListing> for FolderPreview {
    fn from(listing: FolderListing) -> Self {
        Self {
            name: listing.name,
            files: listing.entries.len(),
            size: listing.entries.iter().map(|entry| entry.size).sum(),
            entries: listing
                .entries
                .into_iter()
                .take(MAX_FOLDER_ENTRIES)
                .map(|entry| FolderPreviewEntry {
                    name: entry.name.or_else(|| naming::from_url(&entry.url)).unwrap_or(entry.url),
                    size: entry.size,
                    rel_dir: entry.rel_dir,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MagnetPreview {
    pub name: Option<String>,
    /// Lowercase hex.
    pub info_hash: String,
}

/// Result of a link check (see `LinkInfo` in `docs/API.md`).
#[derive(Debug, Serialize)]
pub struct LinkInfo {
    pub url: String,
    pub kind: LinkKind,
    pub handler: HandlerInfo,
    pub host: Option<String>,
    pub status: LinkStatus,
    pub filename: Option<String>,
    pub size: Option<u64>,
    pub error: Option<String>,
    pub duplicate: Option<Duplicate>,
    pub folder: Option<FolderPreview>,
    /// Links found on a scanned web page.
    pub found: Option<Vec<String>>,
    pub magnet: Option<MagnetPreview>,
    /// What yt-dlp reported about a media link (title, duration, thumbnail, playlist size).
    pub media: Option<MediaInfo>,
}

impl LinkInfo {
    fn new(link: &Link, handler: HandlerInfo) -> Self {
        Self {
            url: link.url.clone(),
            kind: link.kind,
            handler,
            host: None,
            status: LinkStatus::Unknown,
            filename: None,
            size: None,
            error: None,
            duplicate: None,
            folder: None,
            found: None,
            magnet: None,
            media: None,
        }
    }

    fn fail(mut self, status: LinkStatus, error: impl Into<String>) -> Self {
        self.status = status;
        self.error = Some(error.into());
        self
    }
}

/// Checks links concurrently; results keep the order of `links`. Each link has
/// [`LINK_TIMEOUT`] and the whole call [`REQUEST_TIMEOUT`]: links not checked in time come back
/// `unknown`. Web pages are fetched and scanned for links when `scan_pages` is set.
pub async fn check_links(engine: &Engine, links: Vec<Link>, scan_pages: bool) -> anyhow::Result<Vec<LinkInfo>> {
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let duplicates = find_duplicates(engine, &links).await?;
    let mut checked: Vec<(usize, LinkInfo)> = stream::iter(links.into_iter().enumerate())
        .map(|(index, link)| async move {
            let link_deadline = deadline.min(Instant::now() + LINK_TIMEOUT);
            let info = match link.kind {
                LinkKind::Magnet => check_magnet(engine, &link).await,
                LinkKind::Http => check_http(engine, &link, scan_pages, link_deadline).await,
            };
            (index, info)
        })
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;
    checked.sort_unstable_by_key(|(index, _)| *index);
    Ok(checked
        .into_iter()
        .map(|(_, mut info)| {
            info.duplicate = duplicates.get(&info.url).cloned();
            info
        })
        .collect())
}

/// Who would download each link, without contacting its site: media sites, the hosts the accounts
/// list and the plugins are known locally. The status stays `unknown`. For the browser extension,
/// which asks about the page being viewed without having Rustload fetch it.
pub async fn route_links(engine: &Engine, links: Vec<Link>) -> anyhow::Result<Vec<LinkInfo>> {
    let duplicates = find_duplicates(engine, &links).await?;
    let mut routed = Vec::with_capacity(links.len());
    for link in &links {
        let mut info = match link.kind {
            LinkKind::Magnet => check_magnet(engine, link).await,
            LinkKind::Http => {
                let url = link.url.as_str();
                let handler = match media::detect_str(url) {
                    Some(site)
                        if engine.settings.get().media.enabled && super::media::takes_site_link(engine, url).await =>
                    {
                        HandlerInfo::media(site)
                    }
                    _ => HandlerInfo::of(&Handler::select(engine, url).await),
                };
                LinkInfo { host: url_host(url), ..LinkInfo::new(link, handler) }
            }
        };
        info.duplicate = duplicates.get(&info.url).cloned();
        routed.push(info);
    }
    Ok(routed)
}

/// Magnets are processed by the first account able to: nothing to probe.
async fn check_magnet(engine: &Engine, link: &Link) -> LinkInfo {
    let parsed = magnet::parse(&link.url).unwrap_or_default();
    let mut info = match engine.debrid.magnet_account().await {
        Some(account) => LinkInfo::new(link, HandlerInfo::account(HandlerKind::Magnet, &account)),
        None => LinkInfo::new(link, HandlerInfo::plain(HandlerKind::Magnet, "Magnet"))
            .fail(LinkStatus::Unsupported, IntakeError::NoMagnetAccount.to_string()),
    };
    info.filename = parsed.name.clone();
    info.size = parsed.size;
    info.magnet = parsed.info_hash.map(|info_hash| MagnetPreview { name: parsed.name, info_hash });
    info
}

/// Every step stops at `deadline`; what is known by then is returned.
async fn check_http(engine: &Engine, link: &Link, scan_pages: bool, deadline: Instant) -> LinkInfo {
    let url = link.url.as_str();
    // Media sites go to yt-dlp before anything else, like downloads do (`engine::media::routes`).
    if engine.settings.get().media.enabled
        && let Some(site) = media::detect_str(url)
        && timeout_at(deadline, super::media::takes_site_link(engine, url)).await.unwrap_or(true)
    {
        return check_media(engine, link, site, deadline).await;
    }
    // Playlists are read before debrid accounts and hoster plugins see them: a stream goes to
    // yt-dlp and a list stands for its links, wherever they are hosted.
    let listed = m3u::is_candidate_url(url);
    if listed && let Some(info) = check_playlist(engine, link, deadline).await {
        return info;
    }
    let handler = match timeout_at(deadline, Handler::select(engine, url)).await {
        Ok(handler) => handler,
        Err(_) => {
            let info = LinkInfo::new(link, HandlerInfo::of(&Handler::builtin(engine, url)));
            return LinkInfo { host: url_host(url), ..info }.fail(LinkStatus::Unknown, TIMED_OUT);
        }
    };
    let info = LinkInfo { host: url_host(url), ..LinkInfo::new(link, HandlerInfo::of(&handler)) };

    // Folder links are listed by their plugin when they are added, whoever downloads the files.
    if let Some(hoster) = engine.hosters.find(url)
        && let Ok(parsed) = url::Url::parse(url)
    {
        match timeout_at(deadline, hoster.expand(&engine.download_client(), &parsed)).await {
            Ok(Ok(Some(listing))) if listing.entries.is_empty() => {
                return info.fail(LinkStatus::Offline, "the folder is empty");
            }
            Ok(Ok(Some(listing))) => {
                let folder = FolderPreview::from(listing);
                return LinkInfo {
                    status: LinkStatus::Online,
                    filename: folder.name.clone(),
                    size: folder.size,
                    folder: Some(folder),
                    ..info
                };
            }
            Ok(Ok(None)) => {}
            Ok(Err(HosterError::Offline(reason))) => return info.fail(LinkStatus::Offline, reason),
            Ok(Err(error)) => return info.fail(LinkStatus::Unknown, error.to_string()),
            Err(_) => return info.fail(LinkStatus::Unknown, TIMED_OUT),
        }
    }

    match timeout_at(deadline, checker::probe_with(engine, &handler, url)).await {
        Ok(Availability::Online { filename, size, content_type }) => {
            // Recognised by its type only (`…/stream?id=42`).
            if !listed
                && content_type.as_deref().is_some_and(m3u::is_playlist_type)
                && let Some(found) = check_playlist(engine, link, deadline).await
            {
                return found;
            }
            LinkInfo { status: LinkStatus::Online, filename, size, ..info }
        }
        Ok(Availability::Offline(reason)) => info.fail(LinkStatus::Offline, reason),
        Ok(Availability::Unknown) => info,
        Ok(Availability::WebPage) => {
            let found =
                if scan_pages { timeout_at(deadline, scan_page(engine, url)).await.ok().flatten() } else { None };
            LinkInfo { handler: HandlerInfo::plain(HandlerKind::Page, "Web page"), found, ..info }
                .fail(LinkStatus::Unsupported, "this link leads to a web page, not a file")
        }
        Err(_) => info.fail(LinkStatus::Unknown, TIMED_OUT),
    }
}

/// An M3U playlist: a stream checked like a media link, or a list previewed like a folder. `None`
/// when the link turns out not to be a playlist.
async fn check_playlist(engine: &Engine, link: &Link, deadline: Instant) -> Option<LinkInfo> {
    let playlist = timeout_at(deadline, m3u::fetch(&engine.download_client(), &link.url)).await.ok().flatten()?;
    Some(match playlist {
        Playlist::Stream if !engine.settings.get().media.enabled => {
            LinkInfo { host: url_host(&link.url), ..LinkInfo::new(link, HandlerInfo::media(m3u::STREAM)) }
                .fail(LinkStatus::Unsupported, m3u::NEEDS_YTDLP)
        }
        Playlist::Stream => check_media(engine, link, m3u::STREAM, deadline).await,
        Playlist::List(entries) => list_preview(link, entries),
    })
}

fn list_preview(link: &Link, entries: Vec<m3u::Entry>) -> LinkInfo {
    let handler =
        HandlerInfo { kind: HandlerKind::Playlist, id: Some("m3u".into()), name: "M3U".into(), account_id: None };
    let info = LinkInfo { host: url_host(&link.url), ..LinkInfo::new(link, handler) };
    if entries.is_empty() {
        return info.fail(LinkStatus::Offline, "the playlist lists no links");
    }
    let folder = FolderPreview {
        name: m3u::list_name(&link.url),
        files: entries.len(),
        size: None,
        entries: entries
            .into_iter()
            .take(MAX_FOLDER_ENTRIES)
            .map(|entry| FolderPreviewEntry {
                name: entry.title.or_else(|| naming::from_url(&entry.url)).unwrap_or(entry.url),
                size: None,
                rel_dir: String::new(),
            })
            .collect(),
    };
    LinkInfo { status: LinkStatus::Online, folder: Some(folder), ..info }
}

async fn check_media(engine: &Engine, link: &Link, site: Site, deadline: Instant) -> LinkInfo {
    let info = LinkInfo { host: url_host(&link.url), ..LinkInfo::new(link, HandlerInfo::media(site)) };
    let limit = deadline.saturating_duration_since(Instant::now()).saturating_sub(MEDIA_MARGIN);
    // Finding yt-dlp may run it (`--version`): the deadline bounds that too.
    match timeout_at(deadline, media::probe(engine, &link.url, limit)).await {
        Ok(Ok(found)) if found.is_live => info.fail(LinkStatus::Unsupported, "live streams are not supported"),
        Ok(Ok(mut found)) => {
            // The interface only shows how many entries a playlist has.
            found.entries = Vec::new();
            LinkInfo {
                status: LinkStatus::Online,
                filename: found.title.clone(),
                size: found.filesize_approx,
                media: Some(found),
                ..info
            }
        }
        Ok(Err(error @ (MediaError::Missing | MediaError::Disabled))) => {
            info.fail(LinkStatus::Unsupported, error.to_string())
        }
        Ok(Err(MediaError::Unavailable(reason))) => info.fail(LinkStatus::Offline, reason),
        Ok(Err(MediaError::Timeout(_))) | Err(_) => info.fail(LinkStatus::Unknown, TIMED_OUT),
        Ok(Err(error)) => info.fail(LinkStatus::Unknown, error.to_string()),
    }
}

/// Downloadable links of the page at `url`: magnets, file names with a known extension, and
/// links of hosts a plugin or a debrid account handles. `None` if the page cannot be read.
async fn scan_page(engine: &Engine, url: &str) -> Option<Vec<String>> {
    let mut response = match engine.download_client().get(url).send().await.and_then(|r| r.error_for_status()) {
        Ok(response) => response,
        Err(error) => {
            tracing::debug!("cannot scan page: {}", error.without_url());
            return None;
        }
    };
    let page = response.url().clone();
    let mut body = Vec::new();
    // A read error keeps what arrived so far.
    while let Ok(Some(chunk)) = response.chunk().await {
        body.extend_from_slice(&chunk[..chunk.len().min(MAX_PAGE_BYTES - body.len())]);
        if body.len() == MAX_PAGE_BYTES {
            break;
        }
    }
    // Scanning megabytes of HTML takes a while: keep it off the async workers.
    let links =
        tokio::task::spawn_blocking(move || page::extract_links(&String::from_utf8_lossy(&body), &page)).await.ok()?;
    let debrid = engine.settings.get().debrid.enabled;
    let mut found = Vec::new();
    for link in links {
        if found.len() == MAX_FOUND {
            break;
        }
        let wanted = page::is_file_link(&link)
            || engine.hosters.find(&link).is_some()
            || (debrid && !engine.debrid.candidates(&link).await.is_empty());
        if wanted {
            found.push(link);
        }
    }
    Some(found)
}

/// Packages already holding the links: exact URL of a file (fragments included: MEGA keys live
/// there), or same info-hash for magnets. Unfinished packages win over the history.
async fn find_duplicates(engine: &Engine, links: &[Link]) -> anyhow::Result<HashMap<String, Duplicate>> {
    let mut duplicates = HashMap::new();
    let urls: Vec<&str> = links.iter().filter(|l| l.kind == LinkKind::Http).map(|l| l.url.as_str()).collect();
    if !urls.is_empty() {
        for known in db::downloads::find_by_urls(&engine.db, &urls).await? {
            duplicates.entry(known.link.clone()).or_insert_with(|| known.into());
        }
    }
    let magnets: Vec<(&str, String)> = links
        .iter()
        .filter(|l| l.kind == LinkKind::Magnet)
        .filter_map(|l| Some((l.url.as_str(), magnet::parse(&l.url)?.info_hash?)))
        .collect();
    if !magnets.is_empty() {
        for known in db::packages::list_magnets(&engine.db).await? {
            let Some(info_hash) = magnet::parse(&known.link).and_then(|m| m.info_hash) else { continue };
            for (url, _) in magnets.iter().filter(|(_, hash)| *hash == info_hash) {
                duplicates.entry((*url).to_owned()).or_insert_with(|| known.clone().into());
            }
        }
    }
    Ok(duplicates)
}

#[cfg(test)]
mod tests;
