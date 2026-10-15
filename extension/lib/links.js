/** Link parsing and classification, shared by the menus, the paste box and the page picker. */

const HTTP_RE = /^https?:\/\/\S+$/i
export const MAGNET_RE = /^magnet:\?\S+$/i

export function isSendable(url) {
  return typeof url === 'string' && (HTTP_RE.test(url) || MAGNET_RE.test(url))
}

/** Splits free text into http(s) links and magnets, in order, without duplicates. */
export function parseLinks(text) {
  const links = new Set()
  const magnets = new Set()
  for (const token of String(text ?? '').split(/\s+/)) {
    const value = token.trim().replace(/^[<("'[]+|[>)"',;\]]+$/g, '')
    if (MAGNET_RE.test(value)) magnets.add(value)
    else if (HTTP_RE.test(value)) links.add(value)
  }
  return { links: [...links], magnets: [...magnets] }
}

/** Extensions that make a link look like a file worth downloading. */
const FILE_EXTENSIONS = new Set(
  (
    // archives and disk images
    'zip rar 7z tar gz tgz bz2 tbz2 xz txz zst lz lzma cab iso img dmg ' +
    // video
    'mkv mp4 m4v avi mov wmv flv webm mpg mpeg ts m2ts vob ogv 3gp ' +
    // audio
    'mp3 flac aac ogg oga opus wav m4a m4b wma alac ape ' +
    // documents and books
    'pdf epub mobi azw3 cbz cbr djvu ' +
    // software and packages
    'exe msi msix apk xapk ipa deb rpm appimage pkg snap flatpak jar bin run ' +
    // subtitles, torrents, misc
    'srt ass ssa vtt sub torrent nzb'
  ).split(/\s+/),
)

/** File hosters Rustload (or a debrid service) can handle even when the URL shows no extension. */
const HOSTERS = [
  '1fichier.com',
  'alfafile.net',
  'ddownload.com',
  'dropbox.com',
  'drive.google.com',
  'fikper.com',
  'filefactory.com',
  'filespace.com',
  'gofile.io',
  'hitfile.net',
  'katfile.com',
  'krakenfiles.com',
  'mediafire.com',
  'mega.nz',
  'mega.co.nz',
  'nitroflare.com',
  'pixeldrain.com',
  'rapidgator.net',
  'send.cm',
  'turbobit.net',
  'uploaded.net',
  'uploadgig.com',
  'uptobox.com',
  'usersdrive.com',
  'wetransfer.com',
]

/**
 * Media sites Rustload downloads with yt-dlp (videos, playlists, channels, albums…). Keep in sync
 * with the domains of `src/media/sites.rs`.
 */
const MEDIA_SITES = [
  'arte.tv',
  'b23.tv',
  'bandcamp.com',
  'bilibili.com',
  'bsky.app',
  'dai.ly',
  'dailymotion.com',
  'facebook.com',
  'fb.watch',
  'france.tv',
  'instagram.com',
  'kick.com',
  'mixcloud.com',
  'nico.ms',
  'nicovideo.jp',
  'odysee.com',
  'ok.ru',
  'redd.it',
  'reddit.com',
  'rumble.com',
  'soundcloud.com',
  'streamable.com',
  'ted.com',
  'threads.com',
  'threads.net',
  'tiktok.com',
  'twitch.tv',
  'twitter.com',
  'vimeo.com',
  'vk.com',
  'x.com',
  'youtu.be',
  'youtube-nocookie.com',
  'youtube.com',
]

function onSite(host, sites) {
  return sites.some((site) => host === site || host.endsWith(`.${site}`))
}

function safeDecode(text) {
  try {
    return decodeURIComponent(text)
  } catch {
    return text
  }
}

/**
 * Whether a media-site link points at one video or track, rather than a channel, a profile or a
 * playlist that could add hundreds of files.
 */
function isSingleMedia(parsed, segments) {
  const host = parsed.hostname.replace(/^(www|m|mobile)\./, '')
  const path = parsed.pathname
  if (host === 'youtu.be') return segments.length === 1
  if (host.endsWith('youtube.com')) return (path === '/watch' && parsed.searchParams.has('v')) || segments[0] === 'shorts'
  if (host.endsWith('vimeo.com')) return /^\d+$/.test(segments.at(-1) ?? '')
  if (host.endsWith('soundcloud.com')) return segments.length === 2 && segments[1] !== 'sets'
  if (host.endsWith('bandcamp.com')) return segments[0] === 'track'
  if (host.endsWith('reddit.com')) return segments.includes('comments')
  return /\/(video|videos|status|reel|p|clip|comments)\//.test(`${path}/`)
}

/**
 * Describes a link for display and filtering:
 * `{ url, name, host, ext, kind, single }` with kind `magnet` | `file` | `hoster` | `media` | `page`;
 * `single` tells whether a media link is one video (only those are preselected).
 */
export function describeLink(url) {
  if (MAGNET_RE.test(url)) {
    let name = ''
    try {
      name = new URLSearchParams(url.slice(url.indexOf('?') + 1)).get('dn') ?? ''
    } catch {
      name = ''
    }
    return { url, name: name || 'magnet', host: '', ext: 'magnet', kind: 'magnet' }
  }
  let parsed
  try {
    parsed = new URL(url)
  } catch {
    return { url, name: url, host: '', ext: '', kind: 'page' }
  }
  const host = parsed.hostname.replace(/^www\./, '')
  const segments = parsed.pathname.split('/').filter(Boolean)
  const last = safeDecode(segments.at(-1) ?? '')
  const match = last.match(/\.([a-z\d]{1,8})$/i)
  let ext = match ? match[1].toLowerCase() : ''
  // Split archives (.001, .r00…) are grouped under one pseudo-extension.
  if (/^(\d{3}|r\d{2})$/.test(ext)) ext = 'split'
  else if (!FILE_EXTENSIONS.has(ext)) ext = ''
  // `segments.length`: a media site's home page is just a page.
  let kind = 'page'
  if (ext) kind = 'file'
  else if (onSite(host, HOSTERS)) kind = 'hoster'
  else if (segments.length && onSite(host, MEDIA_SITES)) kind = 'media'
  const name = last || host + (parsed.search ? parsed.search : '')
  return { url, name, host, ext, kind, single: kind !== 'media' || isSingleMedia(parsed, segments) }
}
