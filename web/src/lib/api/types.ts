// Mirrors docs/API.md. Keep both in sync.

export type User = { username: string }

export type AuthStatus = {
  auth_enabled: boolean
  setup_required: boolean
  user: User | null
  /** Embedded in the desktop app, which opens it logged in. */
  desktop?: boolean
}

export type System = {
  version: string
  download_root: string
  uptime_secs: number
  extractor: string | null
  /** 7-Zip can be installed from the interface (Windows). */
  extractor_installable: boolean
  installing_extractor: boolean
  paused: boolean
  schedule_open: boolean
  disk: { free: number; total: number } | null
  /** Free space is below `storage.min_free_mib`: nothing new starts until space is freed. */
  disk_low: boolean
}

export type PackageStatus =
  'queued' | 'downloading' | 'paused' | 'debrid' | 'verifying' | 'extracting' | 'completed' | 'failed'

export type DebridProgress = {
  status: string
  progress: number
  speed: number
  seeders: number | null
  size: number | null
}

export type DownloadStatus = 'queued' | 'resolving' | 'downloading' | 'paused' | 'completed' | 'failed' | 'skipped'

export type Download = {
  id: number
  package_id: number
  url: string
  filename: string | null
  size: number | null
  downloaded: number
  status: DownloadStatus
  error: string | null
  via: string | null
  host: string | null
  path: string | null
  attempts: number
  retry_at: number | null
  position: number
  created_at: number
  started_at: number | null
  finished_at: number | null
  speed: number
  eta: number | null
  connections: number
  checksum: Checksum | null
  /** Media links: what yt-dlp reported once the link was checked. */
  media: MediaMeta | null
  /** Live segment map (see `DownloadProgress.map`); absent outside transfers. */
  map?: string | null
  heads?: number[]
}

export type ChecksumAlgo = 'crc32' | 'md5' | 'sha1' | 'sha256' | 'sha512'

export type Checksum = { algo: ChecksumAlgo; status: 'ok' | 'mismatch' }

export type SourceKind = 'links' | 'magnet' | 'torrent'

export type Package = {
  id: number
  name: string
  category_id: number | null
  dest_dir: string | null
  create_subfolder: boolean
  extract: boolean
  delete_archives: boolean
  archive_password: string | null
  source_kind: SourceKind
  status: PackageStatus
  paused: boolean
  error: string | null
  debrid: DebridProgress | null
  extract_progress: number | null
  verify_progress: number | null
  media: MediaOptions | null
  position: number
  created_at: number
  finished_at: number | null
  total_size: number
  downloaded: number
  files: Download[]
}

export type AddOptions = {
  name?: string
  category_id?: number | null
  create_subfolder?: boolean
  extract?: boolean
  delete_archives?: boolean
  archive_password?: string
  paused?: boolean
  media?: MediaOptions
}

export type AddRequest = AddOptions & { links: string }

export type PackagePatch = {
  name?: string
  category_id?: number | null
  create_subfolder?: boolean
  extract?: boolean
  delete_archives?: boolean
  archive_password?: string | null
  media?: MediaOptions
}

export type MoveTarget = 'top' | 'bottom' | 'up' | 'down'

export type Category = {
  id: number
  name: string
  path: string
  resolved_path: string
  extensions: string[]
  is_default: boolean
  position: number
  /** Rule: regular expression matched against the package name, its links and file names. */
  pattern: string
  /** The folder is outside the download root and does not exist (an unmounted volume). */
  missing: boolean
}

export type CategoryInput = {
  name: string
  path: string
  extensions: string[]
  pattern?: string
  is_default?: boolean
}

export type Provider = {
  id: string
  name: string
  website: string
  api_key_url: string
  device_auth: boolean
  magnets: boolean
  /** Debrid services unlock many hosts; hoster accounts are premium accounts of one file host. */
  kind: 'debrid' | 'hoster'
  /** `credentials` = username + password instead of an API key. */
  auth: 'api_key' | 'credentials'
}

export type Account = {
  id: number
  provider: string
  label: string
  enabled: boolean
  position: number
  api_key_masked: string
  username: string | null
  premium: boolean
  premium_until: number | null
  traffic_left: number | null
  last_check: number | null
  last_error: string | null
}

export type DeviceStart = {
  user_code: string
  verification_url: string
  expires_in: number
  poll_token: string
}

export type DevicePoll = {
  status: 'pending' | 'activated' | 'expired'
  account: Account | null
}

export type LogLevel = 'ERROR' | 'WARN' | 'INFO' | 'DEBUG'

export type LogEntry = {
  id: number
  ts_ms: number
  level: LogLevel
  target: string
  message: string
}

export type NotificationEvent =
  'package_finished' | 'package_failed' | 'queue_finished' | 'extract_failed' | 'checksum_failed' | 'disk_low'

export type NotificationKind = 'webhook' | 'discord' | 'telegram' | 'ntfy' | 'gotify'

export type NotificationTarget = {
  id: string
  name: string
  kind: NotificationKind
  enabled: boolean
  url: string
  token: string
  chat_id: string
  events: NotificationEvent[]
}

export type FileExistsPolicy = 'rename' | 'overwrite' | 'skip'

export type Settings = {
  general: {
    /** `''` follows the browser language. */
    language: '' | 'en' | 'fr'
    theme: 'dark' | 'light' | 'system'
  }
  downloads: {
    max_parallel: number
    connections: number
    min_split_size_mib: number
    speed_limit_enabled: boolean
    speed_limit_kib: number
    max_retries: number
    retry_delay_secs: number
    timeout_secs: number
    user_agent: string
    file_exists: FileExistsPolicy
    auto_start: boolean
  }
  schedule: {
    enabled: boolean
    start: string
    end: string
    days: boolean[]
    speed_rules: SpeedRule[]
  }
  storage: {
    create_subfolder: boolean
    /** 0 disables the free-space guard. */
    min_free_mib: number
  }
  postprocess: {
    extract: boolean
    delete_archives: boolean
    passwords: string[]
    verify_checksums: boolean
  }
  debrid: {
    enabled: boolean
    delete_magnets: boolean
  }
  watch: {
    enabled: boolean
    dir: string
    category_id: number | null
  }
  clicknload: {
    enabled: boolean
    paused: boolean
  }
  notifications: {
    targets: NotificationTarget[]
  }
  hooks: {
    package_finished: string
    package_failed: string
  }
  integrations: {
    api_key: string
    metrics: boolean
  }
  media: MediaSettings
  telegram_bot: {
    enabled: boolean
    token: string
    allowed_chats: string[]
  }
}

export type SpeedRule = {
  enabled: boolean
  start: string
  end: string
  days: boolean[]
  limit_kib: number
}

// ---- Media sites (yt-dlp) ----

export type MediaQuality = 'best' | '2160' | '1440' | '1080' | '720' | '480' | '360' | 'audio'

export type MediaOptions = {
  quality: MediaQuality
  audio_format: 'best' | 'm4a' | 'mp3' | 'opus' | 'flac'
  container: 'auto' | 'mp4' | 'mkv'
  subtitles: boolean
  subtitle_langs: string
  embed_metadata: boolean
  embed_thumbnail: boolean
  sponsorblock: 'off' | 'mark' | 'remove'
}

export type MediaSettings = {
  enabled: boolean
  ytdlp_path: string
  ffmpeg_path: string
  auto_update: boolean
  generic_pages: boolean
  cookies_file: string
  extra_args: string
  defaults: MediaOptions
}

export type MediaStatus = {
  enabled: boolean
  ytdlp: { path: string; version: string | null; managed: boolean } | null
  ffmpeg: { path: string; version: string | null } | null
  updating: boolean
  /** ffmpeg can be installed from the interface (Windows). */
  ffmpeg_installable: boolean
  installing_ffmpeg: boolean
  last_update_check: number | null
  /** Latest official yt-dlp release seen, null until checked. */
  latest_version: string | null
}

export type MediaMeta = {
  id: string | null
  title: string | null
  duration_secs: number | null
  thumbnail: string | null
  uploader: string | null
  extractor: string | null
}

export type MediaInfo = {
  title: string | null
  uploader: string | null
  duration_secs: number | null
  thumbnail: string | null
  webpage_url: string | null
  extractor: string | null
  is_live: boolean
  playlist: { title: string | null; count: number } | null
  entries: { url: string; title: string | null; duration_secs: number | null }[]
  filesize_approx: number | null
}

// ---- Link grabber ----

export type LinkHandlerType = 'debrid' | 'hoster' | 'media' | 'direct' | 'page' | 'playlist' | 'magnet'

export type LinkInfo = {
  url: string
  kind: 'http' | 'magnet'
  handler: { type: LinkHandlerType; id: string | null; name: string; account_id: number | null }
  host: string | null
  status: 'online' | 'offline' | 'unknown' | 'unsupported'
  filename: string | null
  size: number | null
  error: string | null
  duplicate: { location: 'queue' | 'history'; package_id: number; package_name: string } | null
  folder: {
    name: string | null
    files: number
    size: number | null
    entries: { name: string; size: number | null; rel_dir: string }[]
  } | null
  found: string[] | null
  magnet: { name: string | null; info_hash: string } | null
  media?: MediaInfo | null
}

// ---- Statistics ----

export type Stats = {
  days: number
  totals: {
    bytes: number
    files: number
    packages: number
    failed: number
    avg_speed: number | null
    peak_speed: number | null
  }
  all_time: { bytes: number; files: number; since: number | null }
  daily: { day: string; bytes: number; files: number; failed: number; peak_speed: number | null }[]
  hosts: { host: string; bytes: number; files: number }[]
  providers: { via: string; bytes: number; files: number }[]
  hours: number[]
}

// ---- System tools ----

export type DiagnosticCheck = {
  id: string
  status: 'ok' | 'warn' | 'error'
  title: string
  detail: string
  hint: string | null
}

export type RestoreResult = {
  settings: boolean
  categories: number
  accounts: number
  packages: number
  /** Parts that were skipped (unknown provider, package that could not be re-added…), in English. */
  errors: string[]
}

// ---- Server-Sent Events payloads ----

export type StatsEvent = {
  speed: number
  active: number
  queued: number
  paused: boolean
  schedule_open: boolean
  speed_limit: number
  queue_remaining: number
  eta: number | null
  disk_free: number | null
  disk_total: number | null
  disk_low: boolean
  speed_rule_active: boolean
}

export type DownloadProgress = {
  id: number
  package_id: number
  status: DownloadStatus
  downloaded: number
  size: number | null
  speed: number
  eta: number | null
  connections: number
  /**
   * 100 characters, one per 1 % slice of the file: `floor(fraction × 9)` as a digit, so `9` is a
   * complete slice. Null when the transfer is single-stream or the size is unknown.
   */
  map: string | null
  /** Write position of each connection, as a fraction of the file size. */
  heads: number[]
}

export type PackageProgress = {
  id: number
  status: PackageStatus
  debrid: DebridProgress | null
  extract_progress: number | null
  verify_progress: number | null
}

export type ProgressEvent = {
  downloads: DownloadProgress[]
  packages: PackageProgress[]
}

export type NoticeEvent = {
  level: 'success' | 'info' | 'warning' | 'error'
  /** Translation key, e.g. "package_finished"; the UI falls back to `message` when unknown. */
  key: string
  params: Record<string, string>
  /** English fallback text. */
  message: string
}
