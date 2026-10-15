//! Curated list of media sites sent to yt-dlp.
//!
//! yt-dlp supports more than a thousand sites, but only links recognised here are routed to it
//! before debrid accounts and hoster plugins: the list is deliberately conservative (video pages,
//! not every page of a site) so that ordinary file links keep their normal path. Other pages can
//! still be tried with yt-dlp through the `media.generic_pages` setting.

use url::Url;

use crate::util::host_matches;

/// A recognised media site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Site {
    /// Stable identifier, e.g. `youtube`.
    pub id: &'static str,
    /// Display name, e.g. `YouTube`.
    pub name: &'static str,
    /// The link lists several videos (playlist, channel, album...): it is expanded into one
    /// download per entry when added.
    pub collection: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Single,
    Collection,
}

use Kind::{Collection, Single};

/// Classifies the path segments (empty ones removed) of a link whose host matched.
type Classifier = fn(&Url, &str, &[&str]) -> Option<Kind>;

struct Rule {
    id: &'static str,
    name: &'static str,
    /// Matching domains, sub-domains included.
    domains: &'static [&'static str],
    classify: Classifier,
}

const RULES: &[Rule] = &[
    Rule {
        id: "youtube",
        name: "YouTube",
        domains: &["youtube.com", "youtube-nocookie.com", "youtu.be"],
        classify: youtube,
    },
    Rule { id: "vimeo", name: "Vimeo", domains: &["vimeo.com"], classify: vimeo },
    Rule { id: "dailymotion", name: "Dailymotion", domains: &["dailymotion.com", "dai.ly"], classify: dailymotion },
    Rule { id: "twitch", name: "Twitch", domains: &["twitch.tv"], classify: twitch },
    Rule { id: "soundcloud", name: "SoundCloud", domains: &["soundcloud.com"], classify: soundcloud },
    Rule { id: "bandcamp", name: "Bandcamp", domains: &["bandcamp.com"], classify: bandcamp },
    Rule { id: "x", name: "X", domains: &["x.com", "twitter.com"], classify: x },
    Rule { id: "tiktok", name: "TikTok", domains: &["tiktok.com"], classify: tiktok },
    Rule { id: "instagram", name: "Instagram", domains: &["instagram.com"], classify: instagram },
    Rule { id: "facebook", name: "Facebook", domains: &["facebook.com", "fb.watch"], classify: facebook },
    Rule { id: "reddit", name: "Reddit", domains: &["reddit.com", "redd.it"], classify: reddit },
    Rule { id: "bilibili", name: "Bilibili", domains: &["bilibili.com", "b23.tv"], classify: bilibili },
    Rule { id: "arte", name: "ARTE", domains: &["arte.tv"], classify: arte },
    Rule { id: "francetv", name: "france.tv", domains: &["france.tv"], classify: francetv },
    Rule { id: "mixcloud", name: "Mixcloud", domains: &["mixcloud.com"], classify: mixcloud },
    Rule { id: "rumble", name: "Rumble", domains: &["rumble.com"], classify: rumble },
    Rule { id: "odysee", name: "Odysee", domains: &["odysee.com"], classify: odysee },
    Rule { id: "streamable", name: "Streamable", domains: &["streamable.com"], classify: streamable },
    Rule { id: "kick", name: "Kick", domains: &["kick.com"], classify: kick },
    Rule { id: "bluesky", name: "Bluesky", domains: &["bsky.app"], classify: bluesky },
    Rule { id: "threads", name: "Threads", domains: &["threads.net", "threads.com"], classify: threads },
    Rule { id: "ted", name: "TED", domains: &["ted.com"], classify: ted },
    Rule { id: "niconico", name: "Niconico", domains: &["nicovideo.jp", "nico.ms"], classify: niconico },
    Rule { id: "vk", name: "VK", domains: &["vk.com"], classify: vk },
    Rule { id: "ok", name: "OK.ru", domains: &["ok.ru"], classify: okru },
];

/// The media site `url` belongs to, if it is a page yt-dlp should handle.
pub fn detect(url: &Url) -> Option<Site> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let segments: Vec<&str> = url.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default();
    RULES.iter().find_map(|rule| {
        let domain = rule.domains.iter().find(|domain| host_matches(&host, domain))?;
        let kind = (rule.classify)(url, &host_label(&host, domain), &segments)?;
        Some(Site { id: rule.id, name: rule.name, collection: kind == Collection })
    })
}

/// `detect` for a link given as text.
pub fn detect_str(link: &str) -> Option<Site> {
    Url::parse(link).ok().as_ref().and_then(detect)
}

/// The matched domain with the sub-domain that matters for routing (`clips.twitch.tv`), without
/// the ones that do not (`www.`, `m.`, `mobile.`).
fn host_label(host: &str, domain: &str) -> String {
    let sub = host.strip_suffix(domain).unwrap_or_default().trim_end_matches('.');
    match sub {
        "" | "www" | "m" | "mobile" | "web" | "old" | "new" => domain.to_owned(),
        sub => format!("{sub}.{domain}"),
    }
}

fn has_query(url: &Url, key: &str) -> bool {
    url.query_pairs().any(|(k, v)| k == key && !v.is_empty())
}

fn is_digits(segment: &str) -> bool {
    !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit())
}

fn youtube(url: &Url, host: &str, s: &[&str]) -> Option<Kind> {
    if host == "youtu.be" {
        return matches!(s, [_]).then_some(Single);
    }
    match s {
        ["watch"] => has_query(url, "v").then_some(Single),
        ["shorts" | "live" | "embed" | "v" | "clip", _, ..] => Some(Single),
        ["playlist"] => has_query(url, "list").then_some(Collection),
        // YouTube Music albums and artists.
        ["browse", _] => Some(Collection),
        ["channel" | "c" | "user", _, ..] => Some(Collection),
        [handle, ..] if handle.starts_with('@') => Some(Collection),
        _ => None,
    }
}

fn vimeo(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    const RESERVED: [&str; 14] = [
        "about",
        "blog",
        "categories",
        "features",
        "help",
        "join",
        "log_in",
        "manage",
        "ondemand",
        "search",
        "settings",
        "upgrade",
        "upload",
        "watch",
    ];
    match s {
        [id, ..] if is_digits(id) => Some(Single),
        ["video", id] | ["channels", _, id] | ["groups", _, "videos", id] if is_digits(id) => Some(Single),
        ["showcase" | "album", _, ..] | ["channels", _] => Some(Collection),
        [user] | [user, "videos"] if !RESERVED.contains(user) => Some(Collection),
        _ => None,
    }
}

fn dailymotion(_: &Url, host: &str, s: &[&str]) -> Option<Kind> {
    if host == "dai.ly" {
        return matches!(s, [_]).then_some(Single);
    }
    match s {
        ["video", _] | ["embed", "video", _] => Some(Single),
        ["playlist", _] => Some(Collection),
        _ => None,
    }
}

fn twitch(_: &Url, host: &str, s: &[&str]) -> Option<Kind> {
    const RESERVED: [&str; 10] =
        ["directory", "downloads", "inventory", "jobs", "p", "search", "settings", "subscriptions", "turbo", "wallet"];
    if host == "clips.twitch.tv" {
        return matches!(s, [_]).then_some(Single);
    }
    match s {
        ["videos", id] if is_digits(id) => Some(Single),
        [_, "clip", _] => Some(Single),
        [channel, "videos"] if !RESERVED.contains(channel) => Some(Collection),
        // A channel page is its live stream: probing reports it as live, which gives a clear error.
        [channel] if !RESERVED.contains(channel) => Some(Single),
        _ => None,
    }
}

fn soundcloud(_: &Url, host: &str, s: &[&str]) -> Option<Kind> {
    const RESERVED: [&str; 18] = [
        "artists",
        "charts",
        "discover",
        "feed",
        "imprint",
        "jobs",
        "messages",
        "mobile",
        "notifications",
        "pages",
        "people",
        "pro",
        "search",
        "settings",
        "stream",
        "terms-of-use",
        "upload",
        "you",
    ];
    if host == "on.soundcloud.com" {
        return matches!(s, [_]).then_some(Single);
    }
    match s {
        [user, ..] if RESERVED.contains(user) => None,
        [_, "sets", _, ..] => Some(Collection),
        [_, "albums" | "tracks" | "reposts" | "likes" | "popular-tracks" | "sets"] | [_] => Some(Collection),
        [_, _] => Some(Single),
        _ => None,
    }
}

fn bandcamp(_: &Url, host: &str, s: &[&str]) -> Option<Kind> {
    match s {
        ["track", _] => Some(Single),
        ["album", _] => Some(Collection),
        // An artist page: `artist.bandcamp.com` or `artist.bandcamp.com/music`.
        [] | ["music"] if host != "bandcamp.com" && host != "daily.bandcamp.com" => Some(Collection),
        _ => None,
    }
}

fn x(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    matches!(s, [_, "status", id, ..] if is_digits(id)).then_some(Single)
}

fn tiktok(_: &Url, host: &str, s: &[&str]) -> Option<Kind> {
    if host == "vm.tiktok.com" || host == "vt.tiktok.com" {
        return matches!(s, [_]).then_some(Single);
    }
    match s {
        [_, "video", id] | ["embed", "v2", id] if is_digits(id) => Some(Single),
        ["t", _] => Some(Single),
        [user] if user.starts_with('@') => Some(Collection),
        _ => None,
    }
}

fn instagram(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    match s {
        ["reel" | "reels" | "p" | "tv", _, ..] | [_, "reel" | "p", _, ..] => Some(Single),
        _ => None,
    }
}

fn facebook(url: &Url, host: &str, s: &[&str]) -> Option<Kind> {
    if host == "fb.watch" {
        return matches!(s, [_]).then_some(Single);
    }
    match s {
        ["watch"] | ["watch", "live"] | ["video.php"] => has_query(url, "v").then_some(Single),
        ["reel", _] | ["share", "v" | "r", _] => Some(Single),
        [_, "videos", rest @ ..] if rest.last().is_some_and(|id| is_digits(id)) => Some(Single),
        _ => None,
    }
}

fn reddit(_: &Url, host: &str, s: &[&str]) -> Option<Kind> {
    if host == "v.redd.it" || host == "redd.it" {
        return matches!(s, [_]).then_some(Single);
    }
    match s {
        ["r" | "user", _, "comments", _, ..] | ["r", _, "s", _] => Some(Single),
        _ => None,
    }
}

fn bilibili(_: &Url, host: &str, s: &[&str]) -> Option<Kind> {
    match (host, s) {
        ("b23.tv", [_]) => Some(Single),
        ("space.bilibili.com", [uid, ..]) if is_digits(uid) => Some(Collection),
        ("bilibili.com", ["video", _] | ["bangumi", "play", _]) => Some(Single),
        _ => None,
    }
}

fn arte(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    match s {
        [_, "videos", id, ..] if id.starts_with("RC-") => Some(Collection),
        [_, "videos", _, ..] => Some(Single),
        _ => None,
    }
}

fn francetv(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    s.last().is_some_and(|last| last.ends_with(".html")).then_some(Single)
}

fn mixcloud(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    const RESERVED: [&str; 10] =
        ["about", "categories", "dashboard", "discover", "live", "pro", "search", "select", "settings", "upload"];
    const LISTS: [&str; 7] = ["favorites", "followers", "following", "listens", "playlists", "stream", "uploads"];
    match s {
        [user, ..] if RESERVED.contains(user) => None,
        [_] | [_, "playlists", _] => Some(Collection),
        [_, list] if LISTS.contains(list) => Some(Collection),
        [_, _] => Some(Single),
        _ => None,
    }
}

fn rumble(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    match s {
        [page] if page.starts_with('v') && page.ends_with(".html") => Some(Single),
        ["embed", _] => Some(Single),
        ["c" | "user", _, ..] => Some(Collection),
        _ => None,
    }
}

fn odysee(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    match s {
        [channel, _] if channel.starts_with('@') => Some(Single),
        [channel] if channel.starts_with('@') => Some(Collection),
        ["$", "embed", ..] => Some(Single),
        [claim] if claim.contains(':') => Some(Single),
        _ => None,
    }
}

fn streamable(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    match s {
        [_] | ["e" | "o", _] => Some(Single),
        _ => None,
    }
}

fn kick(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    const RESERVED: [&str; 6] = ["browse", "categories", "following", "search", "privacy-policy", "terms-of-service"];
    match s {
        [_, "videos" | "clips", _] | ["video", _] => Some(Single),
        // A channel page is its live stream, rejected with a clear error once probed.
        [channel] if !RESERVED.contains(channel) => Some(Single),
        _ => None,
    }
}

fn bluesky(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    matches!(s, ["profile", _, "post", _]).then_some(Single)
}

fn threads(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    matches!(s, [user, "post", _] if user.starts_with('@')).then_some(Single)
}

fn ted(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    matches!(s, ["talks", _]).then_some(Single)
}

fn niconico(_: &Url, host: &str, s: &[&str]) -> Option<Kind> {
    match (host, s) {
        ("nico.ms", [_]) | (_, ["watch", _]) => Some(Single),
        _ => None,
    }
}

fn vk(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    matches!(s, [video] if video.starts_with("video") && video.contains('_')).then_some(Single)
}

fn okru(_: &Url, _: &str, s: &[&str]) -> Option<Kind> {
    matches!(s, ["video" | "videoembed", id] if is_digits(id)).then_some(Single)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(link: &str) -> Option<(&'static str, bool)> {
        detect_str(link).map(|s| (s.id, s.collection))
    }

    #[test]
    fn detects_youtube() {
        assert_eq!(site("https://www.youtube.com/watch?v=jNQXAC9IVRw"), Some(("youtube", false)));
        assert_eq!(site("https://youtube.com/watch?v=abc&list=PL123"), Some(("youtube", false)));
        assert_eq!(site("https://m.youtube.com/watch?v=abc"), Some(("youtube", false)));
        assert_eq!(site("https://music.youtube.com/watch?v=abc"), Some(("youtube", false)));
        assert_eq!(site("https://www.youtube.com/shorts/abc123"), Some(("youtube", false)));
        assert_eq!(site("https://www.youtube.com/live/abc123"), Some(("youtube", false)));
        assert_eq!(site("https://www.youtube-nocookie.com/embed/abc"), Some(("youtube", false)));
        assert_eq!(site("https://youtu.be/jNQXAC9IVRw?t=3"), Some(("youtube", false)));
        assert_eq!(site("https://www.youtube.com/playlist?list=PL123"), Some(("youtube", true)));
        assert_eq!(site("https://music.youtube.com/playlist?list=OLAK5uy"), Some(("youtube", true)));
        assert_eq!(site("https://music.youtube.com/browse/MPREb_abc"), Some(("youtube", true)));
        assert_eq!(site("https://www.youtube.com/@jawed"), Some(("youtube", true)));
        assert_eq!(site("https://www.youtube.com/@jawed/videos"), Some(("youtube", true)));
        assert_eq!(site("https://www.youtube.com/channel/UC4QobU6STFB0P71PMvOGN5A"), Some(("youtube", true)));
        assert_eq!(site("https://www.youtube.com/"), None);
        assert_eq!(site("https://www.youtube.com/watch"), None);
        assert_eq!(site("https://www.youtube.com/results?search_query=cats"), None);
        assert_eq!(site("https://www.youtube.com/playlist"), None);
    }

    #[test]
    fn detects_video_platforms() {
        assert_eq!(site("https://vimeo.com/76979871"), Some(("vimeo", false)));
        assert_eq!(site("https://player.vimeo.com/video/76979871"), Some(("vimeo", false)));
        assert_eq!(site("https://vimeo.com/channels/staffpicks/76979871"), Some(("vimeo", false)));
        assert_eq!(site("https://vimeo.com/showcase/123"), Some(("vimeo", true)));
        assert_eq!(site("https://vimeo.com/someuser"), Some(("vimeo", true)));
        assert_eq!(site("https://vimeo.com/upload"), None);
        assert_eq!(site("https://www.dailymotion.com/video/x8abc"), Some(("dailymotion", false)));
        assert_eq!(site("https://dai.ly/x8abc"), Some(("dailymotion", false)));
        assert_eq!(site("https://www.dailymotion.com/playlist/x6abc"), Some(("dailymotion", true)));
        assert_eq!(site("https://www.twitch.tv/videos/123456"), Some(("twitch", false)));
        assert_eq!(site("https://clips.twitch.tv/FunnySlug"), Some(("twitch", false)));
        assert_eq!(site("https://www.twitch.tv/streamer/clip/FunnySlug"), Some(("twitch", false)));
        assert_eq!(site("https://www.twitch.tv/streamer/videos"), Some(("twitch", true)));
        assert_eq!(site("https://www.twitch.tv/streamer"), Some(("twitch", false)));
        assert_eq!(site("https://www.twitch.tv/directory"), None);
        assert_eq!(site("https://rumble.com/v4abc-some-title.html"), Some(("rumble", false)));
        assert_eq!(site("https://rumble.com/c/SomeChannel"), Some(("rumble", true)));
        assert_eq!(site("https://odysee.com/@chan:1/video-name:2"), Some(("odysee", false)));
        assert_eq!(site("https://odysee.com/@chan:1"), Some(("odysee", true)));
        assert_eq!(site("https://streamable.com/abc12"), Some(("streamable", false)));
        assert_eq!(site("https://kick.com/streamer/videos/0b5a-uuid"), Some(("kick", false)));
        assert_eq!(site("https://kick.com/streamer"), Some(("kick", false)));
        assert_eq!(site("https://www.bilibili.com/video/BV1xx411c7mD"), Some(("bilibili", false)));
        assert_eq!(site("https://b23.tv/abc"), Some(("bilibili", false)));
        assert_eq!(site("https://space.bilibili.com/123456"), Some(("bilibili", true)));
        assert_eq!(site("https://www.arte.tv/fr/videos/110342-000-A/some-title/"), Some(("arte", false)));
        assert_eq!(site("https://www.arte.tv/fr/videos/RC-014035/collection/"), Some(("arte", true)));
        assert_eq!(site("https://www.france.tv/france-2/journal/1234-edition.html"), Some(("francetv", false)));
        assert_eq!(site("https://www.france.tv/france-2/"), None);
        assert_eq!(site("https://www.ted.com/talks/some_talk"), Some(("ted", false)));
        assert_eq!(site("https://www.nicovideo.jp/watch/sm9"), Some(("niconico", false)));
        assert_eq!(site("https://vk.com/video-123_456"), Some(("vk", false)));
        assert_eq!(site("https://ok.ru/video/123456"), Some(("ok", false)));
    }

    #[test]
    fn detects_audio_sites() {
        assert_eq!(site("https://soundcloud.com/artist/track-name"), Some(("soundcloud", false)));
        assert_eq!(site("https://soundcloud.com/artist/sets/album"), Some(("soundcloud", true)));
        assert_eq!(site("https://soundcloud.com/artist"), Some(("soundcloud", true)));
        assert_eq!(site("https://soundcloud.com/artist/tracks"), Some(("soundcloud", true)));
        assert_eq!(site("https://on.soundcloud.com/abc"), Some(("soundcloud", false)));
        assert_eq!(site("https://soundcloud.com/discover"), None);
        assert_eq!(site("https://artist.bandcamp.com/track/song"), Some(("bandcamp", false)));
        assert_eq!(site("https://artist.bandcamp.com/album/record"), Some(("bandcamp", true)));
        assert_eq!(site("https://artist.bandcamp.com/"), Some(("bandcamp", true)));
        assert_eq!(site("https://bandcamp.com/"), None);
        assert_eq!(site("https://www.mixcloud.com/dj/some-mix/"), Some(("mixcloud", false)));
        assert_eq!(site("https://www.mixcloud.com/dj/"), Some(("mixcloud", true)));
        assert_eq!(site("https://www.mixcloud.com/dj/uploads/"), Some(("mixcloud", true)));
    }

    #[test]
    fn detects_social_networks() {
        assert_eq!(site("https://x.com/user/status/1234567890"), Some(("x", false)));
        assert_eq!(site("https://twitter.com/user/status/1234567890/video/1"), Some(("x", false)));
        assert_eq!(site("https://mobile.twitter.com/i/status/1234567890"), Some(("x", false)));
        assert_eq!(site("https://x.com/user"), None);
        assert_eq!(site("https://www.tiktok.com/@user/video/7234567890123456789"), Some(("tiktok", false)));
        assert_eq!(site("https://vm.tiktok.com/ZMabc/"), Some(("tiktok", false)));
        assert_eq!(site("https://www.tiktok.com/@user"), Some(("tiktok", true)));
        assert_eq!(site("https://www.instagram.com/reel/Cabc123/"), Some(("instagram", false)));
        assert_eq!(site("https://www.instagram.com/p/Cabc123/"), Some(("instagram", false)));
        assert_eq!(site("https://www.instagram.com/user/"), None);
        assert_eq!(site("https://www.facebook.com/watch?v=123"), Some(("facebook", false)));
        assert_eq!(site("https://www.facebook.com/page/videos/123456/"), Some(("facebook", false)));
        assert_eq!(site("https://fb.watch/abc/"), Some(("facebook", false)));
        assert_eq!(site("https://www.facebook.com/share/v/abc/"), Some(("facebook", false)));
        assert_eq!(site("https://www.facebook.com/page"), None);
        assert_eq!(site("https://www.reddit.com/r/videos/comments/abc123/title/"), Some(("reddit", false)));
        assert_eq!(site("https://old.reddit.com/r/videos/comments/abc123/"), Some(("reddit", false)));
        assert_eq!(site("https://v.redd.it/abc123"), Some(("reddit", false)));
        assert_eq!(site("https://www.reddit.com/r/videos/"), None);
        assert_eq!(site("https://bsky.app/profile/someone.bsky.social/post/3kabc"), Some(("bluesky", false)));
        assert_eq!(site("https://www.threads.net/@user/post/Cabc"), Some(("threads", false)));
    }

    #[test]
    fn ignores_other_links() {
        assert_eq!(site("https://example.com/video.mp4"), None);
        assert_eq!(site("https://notyoutube.com/watch?v=abc"), None);
        assert_eq!(site("https://mega.nz/file/abc#key"), None);
        assert_eq!(site("ftp://youtube.com/watch?v=abc"), None);
        assert_eq!(site("not a url"), None);
    }
}
