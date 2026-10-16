// Mock responses for the link grabber, media, statistics and system tools endpoints.

import type { DiagnosticCheck, LinkInfo, MediaStatus, Package, Stats } from '$lib/api'

const GiB = 1024 ** 3
const MiB = 1024 ** 2

const DEBRID_HOSTS = ['1fichier.com', 'rapidgator.net', 'nitroflare.com', 'turbobit.net', 'ddownload.com']
const OFFLINE_HOSTS = ['uptobox.com']

function hash(text: string): number {
  let h = 2166136261
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 16777619)
  return h >>> 0
}

function base(url: string): LinkInfo {
  return {
    url,
    kind: url.startsWith('magnet:') ? 'magnet' : 'http',
    handler: { type: 'direct', id: null, name: 'Direct', account_id: null },
    host: null,
    status: 'online',
    filename: null,
    size: null,
    error: null,
    duplicate: null,
    folder: null,
    found: null,
    magnet: null,
    media: null,
  }
}

function lastSegment(url: URL): string | null {
  const last = url.pathname.split('/').filter(Boolean).pop()
  return last ? decodeURIComponent(last) : null
}

/** Simulates `POST /api/links/check` from the shape of each URL. */
export function checkLink(url: string, scanPages: boolean, known: Package[]): LinkInfo {
  const info = base(url)
  const seed = hash(url)
  const dup = known.find((p) => p.files.some((f) => f.url === url))
  if (dup)
    info.duplicate = { location: dup.finished_at ? 'history' : 'queue', package_id: dup.id, package_name: dup.name }

  if (info.kind === 'magnet') {
    const params = new URLSearchParams(url.slice(8))
    const hashParam = (params.get('xt') ?? '').split(':').pop() ?? ''
    info.magnet = { name: params.get('dn'), info_hash: hashParam.toLowerCase() }
    info.filename = params.get('dn')
    info.handler = { type: 'magnet', id: 'alldebrid', name: 'AllDebrid', account_id: 1 }
    return info
  }

  const parsed = new URL(url)
  const host = parsed.hostname.replace(/^www\./, '')
  info.host = host

  if (/(^|\.)youtube\.com$|^youtu\.be$/.test(host)) {
    const id = parsed.searchParams.get('v') ?? lastSegment(parsed) ?? 'eRsGyueVLvQ'
    const list = parsed.searchParams.get('list')
    info.handler = { type: 'media', id: 'youtube', name: 'YouTube', account_id: null }
    info.media = {
      title: list ? 'Blender Studio — Open Movies' : 'Sintel - Open Movie by Blender Foundation',
      uploader: list ? 'Blender Studio' : 'Blender',
      duration_secs: list ? null : 888,
      thumbnail: `https://i.ytimg.com/vi/${id}/mqdefault.jpg`,
      webpage_url: url,
      extractor: 'youtube',
      is_live: false,
      playlist: list ? { title: 'Blender Studio — Open Movies', count: 14 } : null,
      entries: [],
      filesize_approx: list ? null : 412_000_000,
    }
    info.filename = info.media.title
    return info
  }

  // `.m3u8` links are HLS streams, `.m3u` links lists of files (the server reads the content).
  if (parsed.pathname.endsWith('.m3u8')) {
    const segments = parsed.pathname.split('/').filter(Boolean)
    const title = segments.length > 1 ? segments[segments.length - 2] : host
    info.handler = { type: 'media', id: 'hls', name: 'HLS', account_id: null }
    info.media = {
      title,
      uploader: null,
      duration_secs: 596,
      thumbnail: null,
      webpage_url: url,
      extractor: 'generic',
      is_live: false,
      playlist: null,
      entries: [],
      filesize_approx: null,
    }
    info.filename = `${title}.mp4`
    return info
  }

  if (parsed.pathname.endsWith('.m3u')) {
    const titles = ['Overture', 'Night Drive', 'Neon Rain', 'Dawn', 'Last Exit', 'Afterglow']
    info.handler = { type: 'playlist', id: 'm3u', name: 'M3U', account_id: null }
    info.folder = {
      name: lastSegment(parsed)!.replace(/\.m3u$/, ''),
      files: titles.length,
      size: null,
      entries: titles.map((title) => ({ name: title, size: null, rel_dir: '' })),
    }
    return info
  }

  if (host === 'mega.nz' && parsed.pathname.startsWith('/folder')) {
    info.handler = { type: 'hoster', id: 'mega', name: 'MEGA', account_id: null }
    const entries = ['README.txt', 'Day 1/IMG_2041.jpg', 'Day 1/IMG_2042.jpg', 'Day 2/VID_2107.mp4']
    info.folder = {
      name: 'Holiday photos 2026',
      files: 31,
      size: Math.round(1.34 * GiB),
      entries: entries.map((path) => {
        const parts = path.split('/')
        return { name: parts.pop()!, rel_dir: parts.join('/'), size: Math.round((8 + (hash(path) % 40)) * MiB) }
      }),
    }
    return info
  }

  if (OFFLINE_HOSTS.includes(host)) {
    info.handler = { type: 'debrid', id: 'alldebrid', name: 'AllDebrid', account_id: 1 }
    info.status = 'offline'
    info.error = 'File not found on the host'
    info.filename = lastSegment(parsed)
    return info
  }

  // File hosts often hide the name (`https://1fichier.com/?id`): the mock reads it after the id.
  const name = DEBRID_HOSTS.includes(host) ? (parsed.search.split('/')[1] ?? lastSegment(parsed)) : lastSegment(parsed)
  if (!name || !name.includes('.') || name.endsWith('.html')) {
    info.handler = { type: 'page', id: null, name: 'Web page', account_id: null }
    info.status = 'unknown'
    if (scanPages) {
      info.found = [
        'https://1fichier.com/?a1b2c3d4e5/Tears.of.Steel.2012.4K.part1.rar',
        'https://1fichier.com/?f6g7h8i9j0/Tears.of.Steel.2012.4K.part2.rar',
        'https://1fichier.com/?k1l2m3n4o5/Tears.of.Steel.2012.4K.part3.rar',
        `https://${host}/files/readme.txt`,
      ]
    }
    return info
  }

  if (DEBRID_HOSTS.includes(host)) {
    info.handler =
      host === '1fichier.com'
        ? { type: 'debrid', id: '1fichier', name: '1fichier', account_id: 2 }
        : { type: 'debrid', id: 'alldebrid', name: 'AllDebrid', account_id: 1 }
  }
  info.filename = name
  info.size = Math.round((0.15 + (seed % 1000) / 380) * GiB)
  if (host.endsWith('.example')) info.status = 'unknown'
  return info
}

export function mediaStatus(installed: boolean): MediaStatus {
  return {
    enabled: true,
    ytdlp: installed ? { path: '/config/bin/yt-dlp', version: '2026.09.27', managed: true } : null,
    ffmpeg: { path: '/usr/bin/ffmpeg', version: '7.1.2' },
    updating: false,
    last_update_check: Math.floor(Date.now() / 1000) - 7200,
    latest_version: '2026.09.27',
  }
}

/** Plausible statistics with a weekly rhythm, deterministic per day. */
export function stats(days: number): Stats {
  const daily: Stats['daily'] = []
  const today = new Date()
  for (let i = days - 1; i >= 0; i--) {
    const date = new Date(today.getFullYear(), today.getMonth(), today.getDate() - i)
    const day = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
    const r = (hash(day) % 1000) / 1000
    const weekend = date.getDay() === 0 || date.getDay() === 6
    const quiet = r < 0.12
    const bytes = quiet ? 0 : Math.round((weekend ? 18 : 6) * GiB * (0.3 + r * 1.4))
    daily.push({
      day,
      bytes,
      files: quiet ? 0 : Math.round(bytes / (700 * MiB)) + 1,
      failed: r > 0.85 ? 1 + (hash(day + 'f') % 3) : 0,
      peak_speed: quiet ? null : Math.round((60 + r * 50) * MiB),
    })
  }
  const bytes = daily.reduce((s, d) => s + d.bytes, 0)
  const files = daily.reduce((s, d) => s + d.files, 0)
  const hours = Array.from({ length: 24 }, (_, h) => {
    const night = h < 7 ? 1.6 : h >= 20 ? 1.2 : 0.35
    return Math.round((bytes / 24) * night * (0.7 + (hash(`h${h}`) % 100) / 160))
  })
  return {
    days,
    totals: {
      bytes,
      files,
      packages: Math.round(files / 2.4),
      failed: daily.reduce((s, d) => s + d.failed, 0),
      avg_speed: Math.round(41.7 * MiB),
      peak_speed: Math.max(0, ...daily.map((d) => d.peak_speed ?? 0)) || null,
    },
    all_time: { bytes: bytes * 6.3, files: files * 6, since: Math.floor(Date.now() / 1000) - 86400 * 211 },
    daily,
    hosts: [
      { host: '1fichier.com', bytes: bytes * 0.34, files: Math.round(files * 0.3) },
      { host: 'rapidgator.net', bytes: bytes * 0.21, files: Math.round(files * 0.18) },
      { host: 'youtube.com', bytes: bytes * 0.12, files: Math.round(files * 0.31) },
      { host: 'mega.nz', bytes: bytes * 0.11, files: Math.round(files * 0.08) },
      { host: 'releases.ubuntu.com', bytes: bytes * 0.07, files: 6 },
      { host: 'other', bytes: bytes * 0.15, files: Math.round(files * 0.12) },
    ].map((h) => ({ ...h, bytes: Math.round(h.bytes) })),
    providers: [
      { via: 'alldebrid', bytes: Math.round(bytes * 0.52), files: Math.round(files * 0.45) },
      { via: 'direct', bytes: Math.round(bytes * 0.22), files: Math.round(files * 0.16) },
      { via: 'yt-dlp', bytes: Math.round(bytes * 0.12), files: Math.round(files * 0.31) },
      { via: 'mega', bytes: Math.round(bytes * 0.14), files: Math.round(files * 0.08) },
    ],
    hours,
  }
}

export const diagnostics: DiagnosticCheck[] = [
  {
    id: 'download_dir_writable',
    status: 'ok',
    title: 'Download folder is writable',
    detail: '/downloads',
    hint: null,
  },
  {
    id: 'category_folders',
    status: 'ok',
    title: 'Category folders',
    detail: '2 folder(s) outside the download folder, all writable.',
    hint: null,
  },
  { id: 'config_dir_writable', status: 'ok', title: 'Config folder is writable', detail: '/config', hint: null },
  { id: 'disk_space', status: 'ok', title: 'Free space', detail: '1.42 TiB free of 3.64 TiB', hint: null },
  { id: 'extractor', status: 'ok', title: 'Archive extraction', detail: '7-Zip 26.02 (7zz)', hint: null },
  { id: 'media', status: 'ok', title: 'Media tools', detail: 'yt-dlp 2026.09.27, ffmpeg 7.1.2', hint: null },
  {
    id: 'permissions',
    status: 'warn',
    title: 'File ownership',
    detail: 'Running as uid 0, gid 0',
    hint: 'Set PUID and PGID so downloaded files belong to your user.',
  },
  { id: 'timezone', status: 'ok', title: 'Time zone', detail: 'Europe/Paris, 14:32', hint: null },
  { id: 'accounts', status: 'ok', title: 'Accounts', detail: '2 accounts, no errors', hint: null },
  { id: 'auth', status: 'ok', title: 'Sign-in', detail: 'Enabled', hint: null },
  { id: 'internet', status: 'ok', title: 'Internet access', detail: 'Reachable in 48 ms', hint: null },
]

/** A plausible segment map for a multi-connection transfer at `ratio` completion. */
export function segmentMap(ratio: number, connections: number, seed: number): { map: string; heads: number[] } {
  const regions = Math.max(1, connections)
  const cells: number[] = []
  const heads: number[] = []
  for (let r = 0; r < regions; r++) {
    const start = Math.floor((r * 100) / regions)
    const end = Math.floor(((r + 1) * 100) / regions)
    const jitter = 0.82 + ((hash(`${seed}:${r}`) % 100) / 100) * 0.36
    const progress = Math.min(1, ratio * jitter)
    const filled = progress * (end - start)
    for (let c = start; c < end; c++) {
      const f = Math.min(1, Math.max(0, filled - (c - start)))
      cells.push(f >= 1 ? 9 : Math.floor(f * 9))
    }
    if (progress < 1) heads.push((start + filled) / 100)
  }
  return { map: cells.join(''), heads }
}
