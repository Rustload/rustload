//! Adding work: link lists, magnets and `.torrent` files. Shared by the API, the watch folder,
//! and Click'n'Load.

use std::time::Duration;

use serde::Deserialize;

use super::{
    Engine,
    checker::package_name_from_file,
    m3u::{self, Playlist},
    media::{self, Collection},
    naming, remote,
};
use crate::{
    db::{self, downloads::NewDownload, packages::NewPackage},
    debrid::{AccountHandle, DebridError},
    hosters::FolderListing,
    media::{MediaOptions, MediaOptionsPatch, detect_str},
    models::{DownloadStatus, PackageRow, SourceKind, Stage},
    util::url_host,
};

const EXPAND_TIMEOUT: Duration = Duration::from_secs(60);

/// Options common to every way of adding packages. Missing values use the settings defaults.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AddOptions {
    pub name: Option<String>,
    pub category_id: Option<i64>,
    pub create_subfolder: Option<bool>,
    pub extract: Option<bool>,
    pub delete_archives: Option<bool>,
    pub archive_password: Option<String>,
    pub paused: Option<bool>,
    /// Options of media links (YouTube...); missing fields use the settings defaults.
    pub media: Option<MediaOptionsPatch>,
}

impl AddOptions {
    /// The options `package` was added with, its name included.
    pub fn from_package(package: &PackageRow) -> Self {
        Self {
            name: Some(package.name.clone()),
            category_id: package.category_id,
            create_subfolder: Some(package.create_subfolder),
            extract: Some(package.extract),
            delete_archives: Some(package.delete_archives),
            archive_password: package.archive_password.clone(),
            paused: None,
            media: package.media_options().map(MediaOptionsPatch::from),
        }
    }
}

/// What a package is made of, to add it again: its links, or the magnet or `.torrent` file a
/// debrid service downloads first.
pub enum Source {
    Links(Vec<String>),
    Magnet(String),
    Torrent { file_name: String, data: Vec<u8> },
}

impl Source {
    /// A `.torrent` file whose name may be unknown.
    pub fn torrent(file_name: Option<&str>, data: Vec<u8>) -> Self {
        Self::Torrent { file_name: file_name.unwrap_or("file.torrent").to_owned(), data }
    }

    /// What `package` was added from; for a torrent, the copy kept when it was added.
    pub async fn of(engine: &Engine, package: &PackageRow) -> Result<Self, IntakeError> {
        Ok(match package.source_kind {
            SourceKind::Links => {
                let files = db::downloads::list_by_package(&engine.db, package.id).await?;
                Self::Links(files.into_iter().map(|d| d.url).collect())
            }
            SourceKind::Magnet => Self::Magnet(package.source.clone().unwrap_or_default()),
            SourceKind::Torrent => {
                let path = remote::torrent_path(&engine.config.config_dir, package.id);
                let data = tokio::fs::read(&path).await.map_err(IntakeError::TorrentGone)?;
                Self::torrent(package.source.as_deref(), data)
            }
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IntakeError {
    #[error("no link found")]
    NoLinks,
    #[error("magnets and torrents need an enabled debrid account that supports them")]
    NoMagnetAccount,
    #[error("invalid media options: {0}")]
    InvalidMediaOptions(String),
    #[error("the original .torrent file is no longer available")]
    TorrentGone(#[source] std::io::Error),
    #[error("{0}")]
    Debrid(#[from] DebridError),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

/// Links found in free text, deduplicated, in order of appearance.
#[derive(Debug, Default, PartialEq)]
pub struct ParsedLinks {
    pub http: Vec<String>,
    pub magnets: Vec<String>,
}

pub fn parse_links(text: &str) -> ParsedLinks {
    let mut parsed = ParsedLinks::default();
    for token in text.split_whitespace() {
        let token =
            token.trim_matches(|c: char| matches!(c, '"' | '\'' | '<' | '>' | ',' | ';' | '(' | ')' | '[' | ']'));
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("magnet:?") {
            if !parsed.magnets.iter().any(|m| m == token) {
                parsed.magnets.push(token.to_owned());
            }
        } else if (lower.starts_with("http://") || lower.starts_with("https://"))
            && url::Url::parse(token).is_ok()
            && !parsed.http.iter().any(|l| l == token)
        {
            parsed.http.push(token.to_owned());
        } else if token.len() == 40 && token.chars().all(|c| c.is_ascii_hexdigit()) {
            // A bare BitTorrent info-hash.
            let magnet = format!("magnet:?xt=urn:btih:{}", token.to_ascii_lowercase());
            if !parsed.magnets.contains(&magnet) {
                parsed.magnets.push(magnet);
            }
        }
    }
    parsed
}

/// Adds every link of `text`: http(s) links become one package, each magnet its own package.
pub async fn add_text(engine: &Engine, text: &str, options: &AddOptions) -> Result<Vec<i64>, IntakeError> {
    let parsed = parse_links(text);
    if parsed.http.is_empty() && parsed.magnets.is_empty() {
        return Err(IntakeError::NoLinks);
    }
    let mut ids = Vec::new();
    // Playlists, channels and albums of media sites each become their own package.
    let mut links = parsed.http;
    if engine.settings.get().media.enabled && links.iter().any(|l| detect_str(l).is_some_and(|s| s.collection)) {
        let media_options = media_options(engine, options)?;
        let (collections, others) = media::expand_collections(engine, links, &media_options).await;
        for collection in collections {
            ids.push(add_collection(engine, collection, options, &media_options).await?);
        }
        links = others;
    }
    if !links.is_empty() {
        ids.push(add_links(engine, &links, options).await?);
    }
    for magnet in &parsed.magnets {
        ids.push(add_magnet(engine, magnet, options).await?);
    }
    Ok(ids)
}

pub async fn add_links(engine: &Engine, links: &[String], options: &AddOptions) -> Result<i64, IntakeError> {
    if links.is_empty() {
        return Err(IntakeError::NoLinks);
    }
    let mut downloads = Vec::new();
    let mut folder_name = None;
    for url in links {
        if let Some(listing) = expand_folder(engine, url).await {
            folder_name = folder_name.or(listing.name);
            downloads.extend(listing.entries.into_iter().map(|entry| NewDownload {
                host: url_host(&entry.url),
                url: entry.url,
                rel_dir: entry.rel_dir,
                filename: entry.name,
                size: entry.size.map(|s| s as i64),
                account_id: None,
                media: None,
            }));
            continue;
        }
        match read_playlist(engine, url).await {
            Some(Playlist::Stream) => {
                downloads.push(NewDownload { media: Some(m3u::stream_meta(url)), ..NewDownload::link(url.clone()) });
            }
            Some(Playlist::List(entries)) if !entries.is_empty() => {
                folder_name = folder_name.or_else(|| m3u::list_name(url));
                downloads.extend(entries.into_iter().map(|entry| NewDownload::link(entry.url)));
            }
            _ => downloads.push(NewDownload::link(url.clone())),
        }
    }
    if downloads.is_empty() {
        return Err(IntakeError::NoLinks);
    }
    let media = if engine.settings.get().media.enabled
        && downloads.iter().any(|d| d.media.is_some() || detect_str(&d.url).is_some())
    {
        Some(media_options(engine, options)?)
    } else {
        None
    };

    let explicit = clean_name(options.name.as_deref());
    let locked = explicit.is_some() || folder_name.is_some();
    let name = explicit.or(folder_name).unwrap_or_else(|| default_name(&links[0]));
    let package = new_package(engine, name, locked, options, SourceKind::Links, None);
    let files = downloads.len();
    let id = create_package(engine, package, |id| async move {
        db::downloads::insert_many(&engine.db, id, &downloads).await?;
        db::media::set_package_options(&engine.db, id, media.as_ref()).await
    })
    .await?;
    tracing::info!("added package #{id} with {files} file(s)");
    Ok(id)
}

/// Adds a listed playlist: one package named after it, one download per video.
async fn add_collection(
    engine: &Engine,
    collection: Collection,
    options: &AddOptions,
    media: &MediaOptions,
) -> Result<i64, IntakeError> {
    let Collection { url, title, entries } = collection;
    let name = clean_name(options.name.as_deref())
        .or_else(|| title.filter(|t| !t.trim().is_empty()))
        .unwrap_or_else(|| default_name(&url));
    let package = new_package(engine, name, true, options, SourceKind::Links, None);
    let videos = entries.len();
    let id = create_package(engine, package, |id| async move {
        db::media::insert_entries(&engine.db, id, &entries).await?;
        db::media::set_package_options(&engine.db, id, Some(media)).await
    })
    .await?;
    tracing::info!("added playlist {url} as package #{id} with {videos} video(s)");
    Ok(id)
}

/// Adds a package of downloads; `fill` stores its files. A package whose files cannot be stored
/// is removed again.
async fn create_package<F>(
    engine: &Engine,
    package: NewPackage,
    fill: impl FnOnce(i64) -> F,
) -> Result<i64, IntakeError>
where
    F: Future<Output = anyhow::Result<()>>,
{
    let paused = package.paused;
    let id = db::packages::insert(&engine.db, &NewPackage { stage: Stage::Download, ..package }).await?;
    if let Err(error) = fill(id).await {
        // Never leave an empty package behind.
        let _ = db::packages::delete(&engine.db, id).await;
        return Err(error.into());
    }
    if paused {
        db::downloads::set_status_where(&engine.db, id, &[DownloadStatus::Queued], DownloadStatus::Paused).await?;
    }
    engine.events.queue_changed();
    engine.wake();
    Ok(id)
}

/// Media options of a new package: the request's over the settings defaults.
fn media_options(engine: &Engine, options: &AddOptions) -> Result<MediaOptions, IntakeError> {
    media::options_for(&engine.settings.get(), options.media.as_ref()).map_err(IntakeError::InvalidMediaOptions)
}

/// Adds a package again from what it was made of (history, backup restore).
pub async fn add_again(engine: &Engine, source: Source, options: &AddOptions) -> Result<i64, IntakeError> {
    match source {
        Source::Links(links) => add_links(engine, &links, options).await,
        Source::Magnet(magnet) => add_magnet(engine, &magnet, options).await,
        Source::Torrent { file_name, data } => add_torrent(engine, &file_name, data, options).await,
    }
}

pub async fn add_magnet(engine: &Engine, magnet: &str, options: &AddOptions) -> Result<i64, IntakeError> {
    let account = engine.debrid.magnet_account().await.ok_or(IntakeError::NoMagnetAccount)?;
    let remote_id = account.add_magnet(magnet).await?;
    let name = magnet_display_name(magnet).unwrap_or_else(|| "Magnet".into());
    let package = new_package(engine, name, false, options, SourceKind::Magnet, Some(magnet.to_owned()));
    let id = add_transfer(engine, &account, remote_id, package, options).await?;
    tracing::info!("magnet sent to {} as package #{id}", account.label);
    Ok(id)
}

pub async fn add_torrent(
    engine: &Engine,
    file_name: &str,
    data: Vec<u8>,
    options: &AddOptions,
) -> Result<i64, IntakeError> {
    let account = engine.debrid.magnet_account().await.ok_or(IntakeError::NoMagnetAccount)?;
    let file_name = naming::sanitize(file_name);
    let remote_id = account.add_torrent(&file_name, data.clone()).await?;
    let name = file_name.trim_end_matches(".torrent").to_owned();
    let package = new_package(engine, name, false, options, SourceKind::Torrent, Some(file_name));
    let id = add_transfer(engine, &account, remote_id, package, options).await?;

    // Keep the torrent so the transfer can be submitted again after a remote failure.
    let path = remote::torrent_path(&engine.config.config_dir, id);
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Err(error) = tokio::fs::write(&path, data).await {
        tracing::warn!("cannot keep a copy of the torrent: {error}");
    }
    tracing::info!("torrent sent to {} as package #{id}", account.label);
    Ok(id)
}

/// Adds the package of a transfer just sent to `account`. `package` is named after the magnet
/// or torrent, unless the options name it.
async fn add_transfer(
    engine: &Engine,
    account: &AccountHandle,
    remote_id: String,
    mut package: NewPackage,
    options: &AddOptions,
) -> Result<i64, IntakeError> {
    if let Some(name) = clean_name(options.name.as_deref()) {
        package.name = name;
        package.name_locked = true;
    }
    package.remote_account_id = Some(account.id);
    package.remote_id = Some(remote_id);
    let id = db::packages::insert(&engine.db, &package).await?;
    engine.events.queue_changed();
    engine.wake();
    Ok(id)
}

/// Reads a link ending in `.m3u8`/`.m3u`. Anything but a playlist keeps the link as is.
async fn read_playlist(engine: &Engine, link: &str) -> Option<Playlist> {
    if !m3u::is_candidate_url(link) {
        return None;
    }
    m3u::fetch(&engine.download_client(), link).await
}

/// Lists the files of a folder link handled by a hoster plugin. Failures keep the link as is:
/// the error then shows up on the file.
async fn expand_folder(engine: &Engine, link: &str) -> Option<FolderListing> {
    let hoster = engine.hosters.find(link)?;
    let url = url::Url::parse(link).ok()?;
    match tokio::time::timeout(EXPAND_TIMEOUT, hoster.expand(&engine.download_client(), &url)).await {
        Ok(Ok(listing)) => listing.filter(|l| !l.entries.is_empty()),
        Ok(Err(error)) => {
            tracing::warn!("cannot list folder {link}: {error}");
            None
        }
        Err(_) => {
            tracing::warn!("listing folder {link} timed out");
            None
        }
    }
}

fn new_package(
    engine: &Engine,
    name: String,
    name_locked: bool,
    options: &AddOptions,
    source_kind: SourceKind,
    source: Option<String>,
) -> NewPackage {
    let settings = engine.settings.get();
    NewPackage {
        name,
        name_locked,
        category_id: options.category_id,
        create_subfolder: options.create_subfolder.unwrap_or(settings.storage.create_subfolder),
        extract: options.extract.unwrap_or(settings.postprocess.extract),
        delete_archives: options.delete_archives.unwrap_or(settings.postprocess.delete_archives),
        archive_password: options.archive_password.clone().filter(|p| !p.trim().is_empty()),
        source_kind,
        source,
        stage: Stage::Remote,
        remote_account_id: None,
        remote_id: None,
        paused: options.paused.unwrap_or(!settings.downloads.auto_start),
    }
}

fn clean_name(name: Option<&str>) -> Option<String> {
    name.map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned)
}

/// Package name for a link list: the file name if the URL shows one, otherwise the host. Streams
/// are named after what their URL says (`…/big-buck-bunny/master.m3u8`).
fn default_name(url: &str) -> String {
    if m3u::is_candidate_url(url)
        && let Some(name) = m3u::list_name(url)
    {
        return name;
    }
    naming::from_url(url)
        .filter(|name| name.contains('.'))
        .map(|name| package_name_from_file(&name))
        .or_else(|| url_host(url))
        .unwrap_or_else(|| "Package".into())
}

/// `dn` parameter of a magnet link.
fn magnet_display_name(magnet: &str) -> Option<String> {
    super::magnet::parse(magnet)?.name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mixed_text() {
        let text = "Here: https://a.com/f.zip, <https://b.com/x?y=1>\n\
                    magnet:?xt=urn:btih:abc&dn=My+Show https://a.com/f.zip junk\n\
                    0123456789abcdef0123456789abcdef01234567";
        let parsed = parse_links(text);
        assert_eq!(parsed.http, ["https://a.com/f.zip", "https://b.com/x?y=1"]);
        assert_eq!(
            parsed.magnets,
            ["magnet:?xt=urn:btih:abc&dn=My+Show", "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567"]
        );
    }

    #[test]
    fn names_from_magnets_and_urls() {
        assert_eq!(magnet_display_name("magnet:?xt=urn:btih:abc&dn=My+Show%20S01").as_deref(), Some("My Show S01"));
        assert_eq!(magnet_display_name("magnet:?xt=urn:btih:abc"), None);
        assert_eq!(default_name("https://host.com/files/Movie.part1.rar"), "Movie");
        assert_eq!(default_name("https://1fichier.com/?abcdef"), "1fichier.com");
        assert_eq!(default_name("https://cdn.example.com/big-buck-bunny/hls/master.m3u8"), "big-buck-bunny");
    }
}
