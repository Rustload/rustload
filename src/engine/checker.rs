//! Availability check: fetches names and sizes of queued links before they start, and flags dead
//! links early.

use std::time::Duration;

use futures_util::{StreamExt, stream};
use reqwest::header::{self, HeaderMap};

use super::{
    Engine,
    handler::Handler,
    lifecycle::{self, Settled},
    m3u, naming,
    transfer::Probe,
};
use crate::{
    db::{self, packages::PackageUpdate},
    debrid::DebridError,
    hosters::HosterError,
    models::DownloadRow,
    util::url_host,
};

const BATCH: i64 = 16;
const CONCURRENCY: usize = 4;
const TIMEOUT: Duration = Duration::from_secs(20);

pub async fn run(engine: &Engine) {
    loop {
        let batch = match db::downloads::list_unchecked(&engine.db, BATCH).await {
            Ok(batch) if !batch.is_empty() => batch,
            Ok(_) => return,
            Err(error) => {
                tracing::error!("link check: {error:#}");
                return;
            }
        };
        stream::iter(batch).for_each_concurrent(CONCURRENCY, |download| check(engine, download)).await;
        engine.events.queue_changed();
        if engine.shutdown_token().is_cancelled() {
            return;
        }
    }
}

pub(super) enum Availability {
    Online {
        filename: Option<String>,
        size: Option<u64>,
        /// Of a link probed over HTTP.
        content_type: Option<String>,
    },
    Offline(String),
    /// A link handled by nothing but itself answered with a web page instead of a file.
    WebPage,
    Unknown,
}

async fn check(engine: &Engine, download: DownloadRow) {
    if super::media::check(engine, &download).await {
        return;
    }
    // Files coming from a debrid transfer already carry their name and size.
    let availability = if download.account_id.is_some() || engine.live.is_running(download.id) {
        Availability::Unknown
    } else {
        tokio::time::timeout(TIMEOUT, probe(engine, &download.url)).await.unwrap_or(Availability::Unknown)
    };
    // A playlist not recognised when it was added: see `m3u::settle_queued`.
    if let Availability::Online { filename, content_type, .. } = &availability
        && (content_type.as_deref().is_some_and(m3u::is_playlist_type)
            || filename.as_deref().is_some_and(m3u::has_playlist_extension))
        && m3u::settle_queued(engine, &download).await
    {
        return;
    }

    let host = url_host(&download.url);
    let result = match availability {
        Availability::Offline(reason) => {
            tracing::warn!("link offline: {} ({reason})", download.url);
            let marked = db::downloads::mark_offline(&engine.db, download.id, &format!("link offline: {reason}")).await;
            match marked {
                Ok(true) => lifecycle::on_file_settled(engine, download.package_id, Settled::Failed).await,
                Ok(false) => Ok(()),
                Err(error) => Err(error),
            }
        }
        Availability::Online { filename, size, .. } => {
            if let Some(name) = &filename {
                rename_package_from_file(engine, download.package_id, name).await;
            }
            db::downloads::record_check(
                &engine.db,
                download.id,
                filename.as_deref(),
                size.map(|s| s as i64),
                host.as_deref(),
            )
            .await
        }
        Availability::WebPage | Availability::Unknown => {
            db::downloads::record_check(&engine.db, download.id, None, None, host.as_deref()).await
        }
    };
    if let Err(error) = result {
        tracing::warn!("cannot store link check: {error:#}");
    }
}

async fn probe(engine: &Engine, url: &str) -> Availability {
    probe_with(engine, &Handler::select(engine, url).await, url).await
}

/// Name, size and availability of `url` as seen by `handler`, without downloading the file.
pub(super) async fn probe_with(engine: &Engine, handler: &Handler<'_>, url: &str) -> Availability {
    let (target, headers) = match handler {
        Handler::Debrid(account) => {
            return match account.link_info(url).await {
                Ok(Some(info)) => Availability::Online { filename: info.filename, size: info.size, content_type: None },
                Ok(None) => Availability::Unknown,
                Err(DebridError::LinkDown(reason)) => Availability::Offline(reason),
                Err(_) => Availability::Unknown,
            };
        }
        Handler::Hoster(hoster) => {
            let Ok(parsed) = url::Url::parse(url) else { return Availability::Unknown };
            match hoster.resolve(&engine.download_client(), &parsed).await {
                Ok(link) if link.filename.is_some() => {
                    return Availability::Online { filename: link.filename, size: link.size, content_type: None };
                }
                Ok(link) => (link.url, link.headers),
                Err(HosterError::Offline(reason)) => return Availability::Offline(reason),
                Err(_) => return Availability::Unknown,
            }
        }
        Handler::Direct => (url.to_owned(), HeaderMap::new()),
    };

    let request = engine.download_client().get(&target).headers(headers).header(header::RANGE, "bytes=0-0");
    let response = match request.send().await {
        Ok(response) => response,
        Err(_) => return Availability::Unknown,
    };
    let status = response.status();
    if matches!(status.as_u16(), 404 | 410) {
        return Availability::Offline(format!("HTTP {status}"));
    }
    if !status.is_success() {
        return Availability::Unknown;
    }
    let probe = Probe::from_response(response);
    if probe.is_html() {
        // A hoster page: the real name is only known once a plugin or debrid resolves it. Without
        // either, the link leads to no file at all.
        return if matches!(handler, Handler::Direct) { Availability::WebPage } else { Availability::Unknown };
    }
    let filename = probe.filename.or_else(|| naming::from_url(&probe.final_url));
    Availability::Online { filename, size: probe.size, content_type: probe.content_type }
}

/// Gives a package named after its link the name of its first file, once known (by the link check
/// or when the file starts), unless the user named it or its folder exists.
pub(super) async fn rename_package_from_file(engine: &Engine, package_id: i64, filename: &str) {
    let Ok(Some(package)) = db::packages::get(&engine.db, package_id).await else { return };
    if package.name_locked || package.dest_dir.is_some() {
        return;
    }
    let name = package_name_from_file(filename);
    if name != package.name {
        let update = PackageUpdate { name: Some(name), name_locked: Some(true), ..Default::default() };
        let _ = db::packages::update(&engine.db, package_id, update).await;
    }
}

/// `Movie.2024.part01.rar` → `Movie.2024`.
pub fn package_name_from_file(filename: &str) -> String {
    let lower = filename.to_ascii_lowercase();
    let mut stem = filename;
    for suffix in [".tar.gz", ".tar.bz2", ".tar.xz"] {
        if lower.ends_with(suffix) {
            stem = &filename[..filename.len() - suffix.len()];
        }
    }
    if stem.len() == filename.len() {
        stem = naming::split_extension(filename).0;
    }
    let volume = regex_volume().replace(stem, "");
    let trimmed = volume.trim_end_matches(['.', '_', '-', ' ']);
    if trimmed.is_empty() { filename.to_owned() } else { trimmed.to_owned() }
}

fn regex_volume() -> &'static regex::Regex {
    static VOLUME: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VOLUME.get_or_init(|| regex::Regex::new(r"(?i)[._ -]?(part\d+|\d{3}|7z|zip|rar)$").expect("valid regex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_package_names() {
        assert_eq!(package_name_from_file("Movie.2024.part01.rar"), "Movie.2024");
        assert_eq!(package_name_from_file("Album.zip"), "Album");
        assert_eq!(package_name_from_file("backup.7z.001"), "backup");
        assert_eq!(package_name_from_file("data.tar.gz"), "data");
        assert_eq!(package_name_from_file("video.mkv"), "video");
    }
}
