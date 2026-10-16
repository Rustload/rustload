// Seed data for the dev-only mock backend (`pnpm dev:mock`).

import type { Account, Category, Download, LogEntry, Package, Provider, Settings } from '$lib/api'

const now = () => Math.floor(Date.now() / 1000)
const GiB = 1024 ** 3
const MiB = 1024 ** 2

let nextId = 1000
export const id = () => nextId++

export const providers: Provider[] = [
  {
    id: 'alldebrid',
    name: 'AllDebrid',
    website: 'https://alldebrid.com',
    api_key_url: 'https://alldebrid.com/apikeys/',
    device_auth: true,
    magnets: true,
    kind: 'debrid',
    auth: 'api_key',
  },
  {
    id: 'realdebrid',
    name: 'Real-Debrid',
    website: 'https://real-debrid.com',
    api_key_url: 'https://real-debrid.com/apitoken',
    device_auth: true,
    magnets: true,
    kind: 'debrid',
    auth: 'api_key',
  },
  {
    id: 'premiumize',
    name: 'Premiumize',
    website: 'https://www.premiumize.me',
    api_key_url: 'https://www.premiumize.me/account',
    device_auth: false,
    magnets: true,
    kind: 'debrid',
    auth: 'api_key',
  },
  {
    id: 'torbox',
    name: 'TorBox',
    website: 'https://torbox.app',
    api_key_url: 'https://torbox.app/settings',
    device_auth: false,
    magnets: true,
    kind: 'debrid',
    auth: 'api_key',
  },
  {
    id: '1fichier',
    name: '1fichier',
    website: 'https://1fichier.com',
    api_key_url: 'https://1fichier.com/console/params.pl',
    device_auth: false,
    magnets: false,
    kind: 'hoster',
    auth: 'api_key',
  },
  {
    id: 'rapidgator',
    name: 'Rapidgator',
    website: 'https://rapidgator.net',
    api_key_url: 'https://rapidgator.net/profile/index',
    device_auth: false,
    magnets: false,
    kind: 'hoster',
    auth: 'credentials',
  },
]

// A fresh install only has Downloads; the others show what users build: mounted volumes, a rule.
export const categories: Category[] = [
  {
    id: 1,
    name: 'Downloads',
    path: '',
    resolved_path: '/downloads',
    extensions: [],
    is_default: true,
    position: 0,
    pattern: '',
    missing: false,
  },
  {
    id: 2,
    name: 'Movies',
    path: '/movies',
    resolved_path: '/movies',
    extensions: ['mkv', 'mp4', 'avi', 'mov', 'm4v', 'webm'],
    is_default: false,
    position: 1,
    pattern: '',
    missing: false,
  },
  {
    id: 3,
    name: 'TV shows',
    path: '/tv',
    resolved_path: '/tv',
    extensions: [],
    is_default: false,
    position: 2,
    pattern: 'S\\d{2}E\\d{2}',
    missing: false,
  },
  {
    id: 4,
    name: 'Music',
    path: 'music',
    resolved_path: '/downloads/music',
    extensions: ['mp3', 'flac', 'm4a', 'ogg', 'opus'],
    is_default: false,
    position: 3,
    pattern: '',
    missing: false,
  },
]

export const accounts: Account[] = [
  {
    id: 1,
    provider: 'alldebrid',
    label: 'AllDebrid',
    enabled: true,
    position: 0,
    api_key_masked: 'k8Fz••••••••Qe2w',
    username: 'nightowl',
    premium: true,
    premium_until: now() + 86400 * 143,
    traffic_left: null,
    last_check: now() - 600,
    last_error: null,
  },
  {
    id: 2,
    provider: '1fichier',
    label: '1fichier',
    enabled: true,
    position: 1,
    api_key_masked: 'Xa9c••••••••71Lm',
    username: null,
    premium: true,
    premium_until: now() + 86400 * 41,
    traffic_left: Math.round(1.6 * 1024 ** 4),
    last_check: now() - 1800,
    last_error: null,
  },
]

export const settings: Settings = {
  general: { language: '', theme: 'system' },
  downloads: {
    max_parallel: 3,
    connections: 8,
    min_split_size_mib: 4,
    speed_limit_enabled: false,
    speed_limit_kib: 20480,
    max_retries: 5,
    retry_delay_secs: 30,
    timeout_secs: 30,
    user_agent: 'Rustload/0.1',
    file_exists: 'rename',
    auto_start: true,
  },
  schedule: {
    enabled: false,
    start: '01:00',
    end: '08:00',
    days: [true, true, true, true, true, true, true],
    speed_rules: [
      {
        enabled: true,
        start: '08:00',
        end: '19:00',
        days: [true, true, true, true, true, false, false],
        limit_kib: 5120,
      },
    ],
  },
  storage: { create_subfolder: true, min_free_mib: 1024 },
  postprocess: { extract: true, delete_archives: true, passwords: [], verify_checksums: true },
  debrid: { enabled: true, delete_magnets: false },
  watch: { enabled: false, dir: '/watch', category_id: null },
  clicknload: { enabled: false, paused: false },
  notifications: { targets: [] },
  hooks: { package_finished: '', package_failed: '' },
  integrations: { api_key: '9c1f2e7a4b8d6035f1a2c3e4d5b6a7f80e3d4c5b6a7f8e9d0c1b2a3f4e5d6c7b', metrics: false },
  media: {
    enabled: true,
    ytdlp_path: '',
    ffmpeg_path: '',
    auto_update: true,
    generic_pages: false,
    cookies_file: '',
    extra_args: '',
    defaults: {
      quality: '1080',
      audio_format: 'best',
      container: 'auto',
      subtitles: false,
      subtitle_langs: 'en',
      embed_metadata: true,
      embed_thumbnail: true,
      sponsorblock: 'off',
    },
  },
  telegram_bot: { enabled: false, token: '', allowed_chats: [] },
}

export function makeFile(
  packageId: number,
  filename: string,
  size: number | null,
  host: string,
  extra: Partial<Download> = {},
): Download {
  return {
    id: id(),
    package_id: packageId,
    url: `https://${host}/f/${Math.random().toString(36).slice(2, 10)}/${encodeURIComponent(filename)}`,
    filename,
    size,
    downloaded: 0,
    status: 'queued',
    error: null,
    via: 'alldebrid',
    host,
    path: null,
    attempts: 0,
    retry_at: null,
    position: 0,
    created_at: now() - 3600,
    started_at: null,
    finished_at: null,
    speed: 0,
    eta: null,
    connections: 0,
    checksum: null,
    media: null,
    ...extra,
  }
}

export function makePackage(name: string, files: (pkgId: number) => Download[], extra: Partial<Package> = {}): Package {
  const pkgId = id()
  const list = files(pkgId).map((f, i) => ({ ...f, position: i }))
  return {
    id: pkgId,
    name,
    category_id: null,
    dest_dir: null,
    create_subfolder: true,
    extract: true,
    delete_archives: true,
    archive_password: null,
    source_kind: 'links',
    status: 'queued',
    paused: false,
    error: null,
    debrid: null,
    extract_progress: null,
    verify_progress: null,
    media: null,
    position: 0,
    created_at: now() - 3600,
    finished_at: null,
    total_size: list.reduce((s, f) => s + (f.size ?? 0), 0),
    downloaded: 0,
    files: list,
    ...extra,
  }
}

export function seedQueue(): Package[] {
  return [
    makePackage(
      'Blender.Open.Movies.2160p',
      (p) =>
        ['Spring.2019', 'Coffee.Run.2020', 'Sprite.Fright.2021', 'Charge.2022'].map((name, i) =>
          makeFile(p, `${name}.2160p.mkv`, Math.round((2.1 + i * 0.13) * GiB), '1fichier.com'),
        ),
      { category_id: 2, dest_dir: '/movies/Blender.Open.Movies.2160p' },
    ),
    makePackage('ubuntu-24.04.3-desktop-amd64', (p) => [
      makeFile(p, 'ubuntu-24.04.3-desktop-amd64.iso', Math.round(5.9 * GiB), 'releases.ubuntu.com', { via: 'direct' }),
    ]),
    makePackage('Big Buck Bunny 4K', () => [], {
      source_kind: 'magnet',
      status: 'debrid',
      debrid: { status: 'Downloading', progress: 0.42, speed: 38 * MiB, seeders: 87, size: Math.round(8.2 * GiB) },
    }),
    makePackage(
      'Nine Inch Nails - Ghosts I-IV (2008) [FLAC]',
      (p) => [
        makeFile(p, 'Ghosts I-IV.part1.rar', 500 * MiB, 'rapidgator.net'),
        makeFile(p, 'Ghosts I-IV.part2.rar', 212 * MiB, 'rapidgator.net'),
      ],
      { category_id: 4 },
    ),
    makePackage(
      'Broken link',
      (p) => [
        makeFile(p, 'archive.zip', null, 'uptobox.com', {
          status: 'failed',
          error: 'The link is offline (LINK_DOWN)',
          attempts: 5,
        }),
      ],
      { status: 'failed' },
    ),
  ].map((p, i) => ({ ...p, position: i }))
}

export function seedHistory(): Package[] {
  const names = [
    'Blender Open Movie Collection',
    'Debian 13 netinst',
    'Podcast backlog October',
    'Arch Linux 2026.10.01',
    'Sintel 1080p',
    'Tears of Steel 4K',
  ]
  return names.map((name, i) => {
    const finished = now() - (i + 1) * 5400 - i * 86400 * 0.6
    const size = Math.round((0.6 + i * 1.7) * GiB)
    return makePackage(
      name,
      (p) => [
        makeFile(p, `${name.replace(/\s+/g, '.')}.mkv`, size, 'example.org', {
          status: 'completed',
          downloaded: size,
          started_at: finished - 300 - i * 120,
          finished_at: finished,
          path: `/downloads/videos/${name}/${name.replace(/\s+/g, '.')}.mkv`,
          checksum: i % 2 === 0 ? { algo: 'sha256', status: 'ok' } : null,
        }),
      ],
      {
        status: i === 2 ? 'failed' : 'completed',
        error: i === 2 ? 'Extraction failed: wrong password' : null,
        finished_at: finished,
        dest_dir: `/movies/${name}`,
        category_id: 2,
      },
    )
  })
}

const LOG_LINES: [LogEntry['level'], string, string][] = [
  ['INFO', 'rustload::engine', 'Started download of Spring.2019.2160p.mkv with 8 connections'],
  ['DEBUG', 'rustload::engine::segments', 'Split segment 3 at 1.2 GiB (remaining 640 MiB)'],
  ['INFO', 'rustload::debrid::alldebrid', 'Unlocked 1fichier.com link in 412 ms'],
  ['WARN', 'rustload::engine', 'Connection reset by peer, retrying segment 5 (attempt 2/5)'],
  ['ERROR', 'rustload::debrid::alldebrid', 'LINK_DOWN: uptobox.com/abc123 is offline'],
  ['INFO', 'rustload::postprocess', 'Extracted Ghosts I-IV.part1.rar (2 volumes) in 3.1 s'],
]

let logId = 1
export function logEntry(index = Math.floor(Math.random() * LOG_LINES.length)): LogEntry {
  const [level, target, message] = LOG_LINES[index % LOG_LINES.length]
  return { id: logId++, ts_ms: Date.now(), level, target, message }
}

export function seedLogs(): LogEntry[] {
  return Array.from({ length: 120 }, (_, i) => ({ ...logEntry(i), ts_ms: Date.now() - (120 - i) * 9000 }))
}
