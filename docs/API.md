# Rustload HTTP API

All endpoints live under `/api` and speak JSON. The SPA is served from `/`.

Conventions:

- Timestamps are **unix seconds** (`i64`) unless the field name ends in `_ms`.
- Sizes are **bytes**, speeds are **bytes/second**.
- `null` means "unknown / not applicable".
- Errors use a non-2xx status and the body `{ "error": "human readable message", "code": "MACHINE_CODE" }`.
- Requests authenticated with the session cookie must send the header `X-Rustload-Request: 1` for
  every method other than `GET`/`HEAD` (`403 CSRF` otherwise): it proves they come from the web
  interface and not from another site.
- Tools can use the `X-Api-Key` header instead of a session. The API key cannot reach
  `/api/settings*`, `/api/accounts*`, `/api/notifications*`, `/api/auth/*`, `/api/backup*`,
  `/api/integrations/*`, `/api/system/diagnostics`, `/api/media/install`, `/api/media/ffmpeg/install`
  and `/api/extraction/*` (`403 SESSION_REQUIRED`).
- Authentication is a session cookie (`rustload_session`, HttpOnly, SameSite=Strict). Every `/api/*` route
  except `/api/auth/status`, `/api/auth/login`, `/api/auth/setup`, `/api/auth/handoff` and `/api/health` answers
  `401 {code:"UNAUTHORIZED"}` without a valid session (unless auth is disabled server-side).

---

## Auth

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/api/auth/status` | – | `{ auth_enabled: bool, setup_required: bool, user: { username } \| null, desktop: bool }` |
| POST | `/api/auth/setup` | `{ username, password }` | `{ user: { username } }` + cookie. `409 ALREADY_SETUP` if a user exists |
| POST | `/api/auth/login` | `{ username, password, remember?: bool }` | `{ user: { username } }` + cookie. `401 INVALID_CREDENTIALS` |
| POST | `/api/auth/logout` | – | `204` |
| PUT | `/api/auth/password` | `{ current_password, new_password }` | `204`. `400 INVALID_CREDENTIALS` |
| GET | `/api/auth/handoff?code=…&next=…` | – | Desktop app only: `303` to `next` + cookie. `401` for an unknown or used code, `404` on a server |
| POST | `/api/desktop/pair` | `{ browser?: string }` | Desktop app only, for the browser extension: `{ api_key, url }` once allowed in the app (see below) |

Password rules: at least 8 characters.

`desktop` is `true` when Rustload runs inside the desktop app on this computer. Its interface only
answers on `127.0.0.1`, its account has a random password, and the app opens the window through
`/api/auth/handoff` with a single-use code valid for one minute; `next` must be a path of the
interface (anything else redirects to `/`). There, `current_password` may be left out of
`PUT /api/auth/password`: setting a password lets a browser on the same computer sign in.

`/api/desktop/pair` connects the browser extension to the desktop app without copying the API key:
the app asks the person (naming `browser`, as the extension reports it) and the request waits up
to a minute for the answer. Only requests from this computer with a browser-extension `Origin`
(`chrome-extension://`, `moz-extension://`, `safari-web-extension://`) are accepted, and only that
origin may read the answer (CORS, preflight included). Errors: `403 NOT_AN_EXTENSION`,
`403 PAIRING_REFUSED`, `408 PAIRING_TIMEOUT`, `409 PAIRING_IN_PROGRESS` (the app is already
asking), `503 PAIRING_UNAVAILABLE` (the app cannot ask yet), `404` on a server.
The key is the one in *Settings › Integrations*; regenerating it disconnects the extension.

## System / queue control

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/api/system` | – | `System` |
| POST | `/api/queue/pause` | – | `System` (global pause: active downloads stop, nothing new starts) |
| POST | `/api/queue/resume` | – | `System` |
| POST | `/api/extraction/7zip/install` | – | `System` once 7-Zip 26.04 (`7z.exe` and `7z.dll`, RAR support included) is installed in `<config>/bin` (session only, Windows). The official installer and the standalone `7zr.exe` that unpacks it come from 7-Zip's GitHub releases and are checked against SHA-256 checksums pinned in Rustload. `409 UPDATE_IN_PROGRESS`, `400 UNSUPPORTED_PLATFORM` (other systems: use their package manager), `502 INSTALL_FAILED` |
| GET | `/api/health` | – | `{ status: "ok", version }` — **public**, no authentication. `503 { status: "error", version, error, code: "UNHEALTHY" }` when the database does not answer |

```ts
type System = {
  version: string
  download_root: string        // e.g. "/downloads"
  uptime_secs: number
  extractor: string | null     // "7zz" | "7z" | null (null = only .zip supported natively)
  extractor_installable: boolean  // 7-Zip can be installed with POST /api/extraction/7zip/install (Windows)
  installing_extractor: boolean   // that installation is running
  paused: boolean              // global pause
  schedule_open: boolean       // false = outside the allowed time window
  disk: { free: number, total: number } | null
  disk_low: boolean            // free space below storage.min_free_mib: transfers are held back
  media: {                     // last detection of the media tools; null until the first one
    ytdlp: string | null       // yt-dlp version, null = not found
    ffmpeg: boolean
  } | null
}
```

### Diagnostics

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/api/system/diagnostics` | – | `{ checks: DiagnosticCheck[] }` (session only, answers in under 3 s) |

```ts
type DiagnosticCheck = {
  id: string                       // stable, see below; translate with it
  status: "ok" | "warn" | "error"
  title: string                    // English fallback, e.g. "Disk space"
  detail: string                   // English fallback, e.g. "120 GiB free of 2 TiB (6%)."
  hint: string | null              // English fallback: what to do (may be present on "ok" too)
}
```

| `id` | Checks |
|---|---|
| `download_dir_writable` | A file can be created in the download root (`error` otherwise) |
| `config_dir_writable` | Same for the config folder |
| `disk_space` | Free space: `error` below 1 GiB, `warn` below 5 % or below what the queue still needs |
| `extractor` | 7-Zip found and its version (`warn`: ZIP only, or the binary does not run) |
| `media` | yt-dlp and ffmpeg found, with their versions (`warn`: yt-dlp missing, or ffmpeg missing) |
| `permissions` | uid/gid/umask on Linux and whether `PUID`/`PGID` were applied (`warn` when running as root) |
| `timezone` | `TZ` and the local time (`warn`: unknown zone, or `TZ` unset while the schedule is enabled; the desktop app uses the computer's time zone) |
| `accounts` | Enabled debrid accounts whose last check failed (`warn`) |
| `auth` | `warn` when auth is disabled and the request did not come through a reverse proxy (`X-Forwarded-*`/`Forwarded` headers), or when HTTPS is used without `RUSTLOAD_SECURE_COOKIES` |
| `internet` | One HEAD request to `https://www.cloudflare.com/cdn-cgi/trace` (`warn` if it fails) |
| `clock` | System clock compared with that response's `Date` header (`warn` above 60 s, or when it could not be checked) |

Checks run in this order; more may be added (unknown ids should be displayed with their English
texts).

## Monitoring — `GET /metrics`

Prometheus metrics in the text exposition format (`text/plain; version=0.0.4`). The endpoint lives
outside `/api` and answers `404` unless `settings.integrations.metrics` is `true`. It authenticates
with the API key, sent as `Authorization: Bearer <key>` or `X-Api-Key: <key>` (`401` otherwise; no
authentication when auth is disabled server-side).

| Metric | Type | Labels | Meaning |
|---|---|---|---|
| `rustload_up` | gauge | – | Always `1` |
| `rustload_build_info` | gauge | `version` | Always `1` |
| `rustload_download_speed_bytes` | gauge | – | Current total speed, bytes/s |
| `rustload_downloads` | gauge | `state` = `active` \| `queued` \| `paused` \| `failed` | Files of unfinished packages: transferring, waiting for a slot, paused, failed |
| `rustload_queue_remaining_bytes` | gauge | – | Bytes left in active and queued files |
| `rustload_disk_free_bytes`, `rustload_disk_total_bytes` | gauge | – | File system of the download root (absent if unknown) |
| `rustload_downloaded_bytes_total` | counter | – | Bytes of completed files (statistics, all time) |
| `rustload_files_completed_total` | counter | – | Completed files |
| `rustload_files_failed_total` | counter | – | Files that failed for good |
| `rustload_account_traffic_left_bytes` | gauge | `provider`, `account_id` | Enabled accounts reporting a traffic limit |
| `rustload_account_premium_until_seconds` | gauge | `provider`, `account_id` | Enabled accounts with a premium end date, unix time |
| `rustload_uptime_seconds` | gauge | – | Seconds since start |

## Packages (the queue)

A **package** groups one or more files (a set of links pasted together, or the content of one magnet/torrent).
It owns the destination folder and post-processing options. Unfinished packages are the *queue*; finished
packages are the *history*.

```ts
type PackageStatus =
  | "queued"       // waiting for a download slot
  | "downloading"  // at least one file is transferring
  | "paused"       // paused by the user
  | "debrid"       // magnet/torrent being processed remotely by the debrid service
  | "verifying"    // files are checked against checksums (before extraction)
  | "extracting"   // post-processing
  | "completed"
  | "failed"       // at least one file failed permanently, or debrid/extraction error

type Package = {
  id: number
  name: string
  category_id: number | null    // null = automatic (chosen from file extensions)
  dest_dir: string | null       // absolute folder, resolved when the first file starts
  create_subfolder: boolean
  extract: boolean
  delete_archives: boolean
  archive_password: string | null
  source_kind: "links" | "magnet" | "torrent"
  status: PackageStatus
  paused: boolean
  error: string | null
  debrid: DebridProgress | null        // only for magnet/torrent packages
  extract_progress: number | null      // 0..1 while extracting
  verify_progress: number | null       // 0..1 while verifying checksums
  position: number
  created_at: number
  finished_at: number | null
  total_size: number                   // sum of known file sizes
  downloaded: number
  media: MediaOptions | null           // null when the package has no media link (see Media sites)
  files: Download[]
}

type DebridProgress = {
  status: string        // provider text, e.g. "Downloading", "Ready", "Upload fail"
  progress: number      // 0..1
  speed: number         // remote speed, bytes/s
  seeders: number | null
  size: number | null
}

type DownloadStatus =
  | "queued" | "resolving" | "downloading" | "paused" | "completed" | "failed" | "skipped"

type Download = {
  id: number
  package_id: number
  url: string
  filename: string | null
  size: number | null
  downloaded: number
  status: DownloadStatus
  error: string | null
  via: string | null               // "direct", the Provider.id that unlocked the link, a hoster plugin, or "yt-dlp"
  host: string | null              // e.g. "1fichier.com"
  path: string | null              // absolute final path once completed
  attempts: number
  retry_at: number | null          // set while waiting for an automatic retry
  position: number
  created_at: number
  started_at: number | null
  finished_at: number | null
  // live values (0 / null when not active)
  speed: number
  eta: number | null               // seconds
  connections: number              // always 1 for yt-dlp downloads
  media: MediaMeta | null          // media links: what yt-dlp reported (title, duration...)
  map: string | null               // segment map, see below; null unless a multi-connection capable
                                   // transfer (ranges supported, size known) is running
  heads: number[]                  // write position of each active connection, 0..1 of the size
  checksum: Checksum | null        // null = not verified (no checksum known, or not finished yet)
}

type Checksum = {
  algo: "crc32" | "md5" | "sha1" | "sha256" | "sha512"
  status: "ok" | "mismatch"        // "mismatch": the file was deleted and is downloaded again once,
                                   // then fails with error "checksum mismatch (<algo>)"
}
```

**Segment map.** `map` is exactly 100 characters: character *i* describes the *i*-th 1 % slice of
the file as the digit `floor(completed fraction × 9)` — `"9"` means the slice is complete, `"0"`…`"8"`
partially downloaded. `heads` (sorted, 4 decimals) shows where each connection is writing; draw them
over the map to see connections progress and split work.

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/api/packages` | – | `{ packages: Package[] }` (queue, ordered) |
| POST | `/api/packages` | `AddRequest` | `{ packages: Package[] }`. `400 NO_LINKS`, `400 UNKNOWN_CATEGORY`, `400 NO_MAGNET_ACCOUNT` (magnets without an account able to process them) |
| POST | `/api/packages/batch` | `{ packages: AddRequest[] }` (1..=100) | `{ packages: Package[] }`: every package created, in request order. Every request is validated first (same errors as above, message prefixed with `package N: `; `400 INVALID_BATCH` for 0 or more than 100), then added in order. A debrid error while sending a magnet stops the batch; packages added before stay |
| POST | `/api/packages/torrent` | multipart: one or more `file` parts (.torrent) + `options` part (JSON `AddOptions`) | `{ packages: Package[] }` |
| PATCH | `/api/packages/:id` | `PackagePatch` | `Package` |
| DELETE | `/api/packages/:id?files=true` | – | `204`. Partial files are always removed; completed files only with `files=true` |
| POST | `/api/packages/:id/pause` | – | `Package` |
| POST | `/api/packages/:id/resume` | – | `Package` |
| POST | `/api/packages/:id/retry` | – | `Package` (re-queues failed files) |
| POST | `/api/packages/:id/extract` | – | `Package` (re-runs extraction). `409 EXTRACT_NOT_READY` while files are still downloading |
| POST | `/api/packages/:id/move` | `{ to: "top" \| "bottom" \| "up" \| "down" }` | `204` |
| POST | `/api/packages/reorder` | `{ ids: number[] }` | `204` |
| POST | `/api/downloads/:id/pause` | – | `204` |
| POST | `/api/downloads/:id/resume` | – | `204` |
| POST | `/api/downloads/:id/retry` | – | `204` |
| DELETE | `/api/downloads/:id` | – | `204` |

```ts
type AddOptions = {
  name?: string                   // package name; default derived from the first file / magnet name
  category_id?: number | null     // null or missing = automatic by extension
  create_subfolder?: boolean      // default: settings.storage.create_subfolder
  extract?: boolean               // default: settings.postprocess.extract
  delete_archives?: boolean       // default: settings.postprocess.delete_archives
  archive_password?: string
  paused?: boolean                // add without starting
  media?: Partial<MediaOptions>   // media links only; missing fields use settings.media.defaults
}
type AddRequest = AddOptions & {
  links: string                   // raw text; one link per line/whitespace. http(s) links are grouped
                                  // into one package, each magnet becomes its own package, and each
                                  // media playlist/channel/album its own package (one file per video)
}
type PackagePatch = {
  name?: string
  category_id?: number | null     // 409 DEST_LOCKED once a file has started
  create_subfolder?: boolean      // 409 DEST_LOCKED once a file has started
  extract?: boolean
  delete_archives?: boolean
  archive_password?: string | null
  media?: Partial<MediaOptions>   // missing fields keep their value; applies to files not started yet
}
```

`POST /api/packages` (when media links are added) and `PATCH /api/packages/:id` answer
`400 INVALID_MEDIA_OPTIONS` when `media.subtitle_langs` is not a list of language codes.

## Link grabber

Links are checked before they are added: who will download them, whether they are online, their
name and size, duplicates, folder contents. Nothing is added by the check; add the links the user
keeps with `POST /api/packages/batch` (folder links and M3U lists are added as is and expanded
when added).

| Method | Path | Body | Response |
|---|---|---|---|
| POST | `/api/links/check` | `{ urls: string[], scan_pages?: boolean, probe?: boolean }` | `{ results: LinkInfo[] }` |

`urls`: 1 to 50 http(s) URLs or magnet URIs (`400 NO_LINKS`, `400 TOO_MANY_LINKS`, `400 INVALID_LINK`
for anything else). Results come in the same order. Links are checked 8 at a time, each for at most
20 s, the whole request for at most 30 s: links not checked in time come back `unknown` with
`error: "the check took too long"`. With `scan_pages: true`, web pages are fetched (5 MiB at most)
and their downloadable links returned in `found`. With `probe: false`, nothing is contacted: each
result only tells who would download the link (`handler`, from the media sites, the hosts listed by
the accounts and the plugins) and `duplicate`, with `status: "unknown"`; the browser extension asks
this about the page being viewed.

```ts
type LinkInfo = {
  url: string                      // as sent, trimmed
  kind: "http" | "magnet"
  handler: {
    type: "debrid" | "hoster" | "direct" | "page" | "magnet" | "media" | "playlist"   // more types may be added
    id: string | null              // Provider.id ("alldebrid"), hoster plugin id ("mega"), media site id
                                   // ("youtube", "hls" for HLS streams), "m3u" for lists; null for direct/page
    name: string                   // "AllDebrid", "MEGA", "Direct", "Web page", "Magnet" (no account), "HLS", "M3U"
    account_id: number | null      // debrid account used first (others follow on failure)
  }
  host: string | null              // "1fichier.com"; null for magnets
  status: "online" | "offline" | "unknown" | "unsupported"
  filename: string | null          // file name, folder name, or magnet dn
  size: number | null              // bytes; folder total when every file size is known; magnet xl
  error: string | null             // why offline/unsupported/unknown, when known
  duplicate: { location: "queue" | "history", package_id: number, package_name: string } | null
  folder: {
    name: string | null
    files: number                  // total number of files
    size: number | null            // null unless every file size is known
    entries: { name: string, size: number | null, rel_dir: string }[]   // the first 200 files
  } | null
  found: string[] | null           // scan_pages only, for handler.type == "page": absolute links, max 1000;
                                   // null when not scanned or the page could not be read
  magnet: { name: string | null, info_hash: string } | null   // info_hash: lowercase hex (v1)
  media: MediaInfo | null          // media links: yt-dlp's answer, `entries` left empty (see Media sites)
}
```

- **Media sites** (YouTube, Vimeo...) are handled by yt-dlp when *Settings › Video sites* is enabled:
  `type: "media"`, `id`/`name` of the site, probed with yt-dlp (title as `filename`, estimated
  `size`, `media`); unavailable videos are `offline`, live streams `unsupported`. Without yt-dlp,
  a link that a debrid account or hoster plugin supports gets that handler (as downloads do), the
  others are `type: "media"`, `unsupported` (yt-dlp missing).
- **Handler** follows the download order: magnets go to the first enabled account able to process
  them (`unsupported` without one; magnets are not probed, so `unknown` otherwise). http(s) links go
  to the first enabled debrid account supporting the host (when *Use debrid accounts for supported
  hosts* is on in *Settings › Accounts*), then to a built-in hoster plugin, else they are
  downloaded directly. A direct link answering with
  an HTML page is `type: "page"`, `status: "unsupported"`: scan it instead of adding it.
- **Folders** (MEGA, Google Drive, MediaFire, Pixeldrain lists, Gofile) are listed by their plugin
  whoever downloads the files: `status: "online"` with `folder`, or `offline` when gone or empty.
- **M3U playlists** (links ending in `.m3u8`/`.m3u`, or served as `application/vnd.apple.mpegurl`,
  `audio/x-mpegurl`...) are read before debrid accounts and hoster plugins. An HLS stream is
  `type: "media"`, `id: "hls"`, `name: "HLS"`, checked like a media link (title derived from the
  address as `filename`, `media`); `unsupported` when video sites are disabled. A `.m3u8` link that
  cannot be read (`403`...) is treated as a stream: yt-dlp may get it with the right headers. Any
  other playlist is `type: "playlist"`, `id: "m3u"`, `name: "M3U"`: `status: "online"`, `filename`
  and `size` null, `folder` with the list name and its links (`#EXTINF` title or file name, `size`
  null, at most 200 shown out of 1000 kept), or `offline` when it lists no link. Lists are added
  as their links, like folders.
- **Duplicates**: same URL as a file already in a package (exact match, fragment included), or same
  info-hash as a magnet package. Unfinished packages (`queue`) win over the `history`.
- **Page scan** keeps magnets, links of hosts handled by a plugin or an enabled debrid account, and
  links whose file name ends with an archive (incl. `.partN.rar`, `.001`, `.r00`), disk image,
  installer/package, video, audio, ebook/PDF, `.torrent`, `.m3u8` or `.m3u` extension; relative links are resolved
  against the final page address and `<base href>`, duplicates removed, document order kept.

## History

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/api/history?q=&limit=50&offset=0` | – | `{ items: Package[], total: number }` (newest first) |
| DELETE | `/api/history` | – | `204` (forget all entries, files stay on disk) |
| DELETE | `/api/history/:id?files=true` | – | `204` |
| POST | `/api/history/:id/redownload` | – | `{ packages: Package[] }` |

## Statistics

Completed files are counted when they finish, in local days (the system time zone, `TZ` in Docker). Statistics do not
depend on the history: clearing it keeps them.

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/api/stats?days=30` | – | `Stats` over the last `days` days including today (`1..365`, clamped) |

```ts
type Stats = {
  days: number
  totals: {                     // over the range
    bytes: number
    files: number               // completed files (each once, even when downloaded again)
    packages: number            // packages completed without error (each once, even when extracted again)
    failed: number              // files that failed for good
    avg_speed: number | null    // bytes ÷ transfer seconds, null without transfers
    peak_speed: number | null   // highest total speed of the range (sampled every second)
  }
  all_time: { bytes: number, files: number, since: number | null }  // since = first recorded file
  daily: {                      // every day of the range, zero-filled, oldest first
    day: string                 // "YYYY-MM-DD"
    bytes: number
    files: number
    failed: number
    peak_speed: number | null
  }[]
  hosts: { host: string, bytes: number, files: number }[]   // top 8 by bytes, then "other" (the rest);
                                                            // "" = unknown host
  providers: { via: string, bytes: number, files: number }[] // all, by bytes; via = "direct",
                                                             // Provider.id or hoster plugin name
  hours: number[]               // 24 entries: bytes completed per local hour of the day
}
```

## Categories (destination folders)

```ts
type Category = {
  id: number
  name: string                // "Downloads" on a new installation, then the user's own
  path: string                // relative to download_root, or absolute
  resolved_path: string       // absolute path actually used
  extensions: string[]        // lowercase, without dot. Used by automatic mode
  pattern: string             // rule: case-insensitive regular expression ("" = none), see below
  is_default: boolean         // fallback when no rule nor extension matches
  position: number            // rules are tried in this order
  missing: boolean            // absolute folder that does not exist (unmounted volume)
}
```

Automatic mode takes the first category whose `pattern` matches the package name, one of its links
or file names, then the one whose `extensions` match the most file names, then the default. Files
sent to a `missing` folder fail until it exists. The default category is the download root on a new
installation.

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/api/categories` | – | `{ categories: Category[], download_root: string }` |
| POST | `/api/categories` | `{ name, path, extensions: string[], pattern?: string, is_default?: bool }` | `Category`. `400 INVALID_PATTERN` for an invalid rule (at most 500 characters) |
| PUT | `/api/categories/:id` | same as POST | `Category` |
| DELETE | `/api/categories/:id` | – | `204`. `409 DEFAULT_CATEGORY` for the default one |
| POST | `/api/categories/reorder` | `{ ids: number[] }` | `204` |

## Accounts (debrid services and premium hosters)

Several debrid providers are supported, as well as the premium accounts of some hosters (through
their official APIs). For a link, the accounts supporting its host are tried in this order: the
host's own premium account, debrid services in `position` order (lowest first), the host's own
account without premium; accounts put aside after a quota or temporary error come last. If none
can unlock it, the built-in plugins and the direct download follow. A hoster account only handles
links of its own site.

```ts
type Provider = {
  id: string                  // debrid: "alldebrid" | "realdebrid" | "premiumize" | "debridlink" | "torbox"
                              // hosters: "1fichier" | "rapidgator" | "keep2share" | "fileboom" | "tezfiles"
  name: string                // "AllDebrid"
  website: string             // "https://alldebrid.com"
  api_key_url: string         // page where the user finds their API key ("credentials" providers: the site)
  device_auth: boolean        // supports "connect with a code" login (no key copy/paste)
  magnets: boolean            // can process magnets / .torrent files (always false for hosters)
  kind: "debrid" | "hoster"   // hoster = premium account of one file hoster, only for its own links
  auth: "api_key" | "credentials" // what the user enters: an API key, or a username (e-mail) + password
}

type Account = {
  id: number
  provider: string            // Provider.id
  label: string
  enabled: boolean
  position: number
  api_key_masked: string      // "abcd••••wxyz"; always "••••" for "credentials" providers (password)
  username: string | null     // account name reported by the provider, else the login username
  premium: boolean
  premium_until: number | null
  traffic_left: number | null // bytes, null = unlimited/unknown
  last_check: number | null
  last_error: string | null
}
```

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/api/accounts` | – | `{ accounts: Account[], providers: Provider[] }` |
| POST | `/api/accounts` | `{ provider, label?, api_key?, username?, password? }` | `Account` (see below) |
| PUT | `/api/accounts/:id` | `{ label?, enabled?, api_key?, username?, password? }` | `Account` (new secrets are validated like on creation) |
| DELETE | `/api/accounts/:id` | – | `204` |
| POST | `/api/accounts/:id/check` | – | `Account` (refreshes premium status) |
| POST | `/api/accounts/reorder` | `{ ids: number[] }` | `204` |
| POST | `/api/accounts/device/:provider/start` | – | `{ user_code, verification_url, expires_in, poll_token }` |
| POST | `/api/accounts/device/:provider/poll` | `{ poll_token }` | `{ status: "pending" \| "activated" \| "expired", account: Account \| null }` |

Creating an account: `api_key` is required for `auth: "api_key"` providers, `username` and
`password` for `auth: "credentials"` ones (empty strings count as absent). The credentials are
checked with the provider before the account is stored:

- `400 INVALID_API_KEY` — missing or rejected API key, or username/password sent to an API-key provider;
- `400 INVALID_CREDENTIALS` — missing or rejected username/password (the message tells why, e.g. a
  captcha or two-factor authentication the site requires), or an API key sent to such a provider;
- `400 UNKNOWN_PROVIDER`, `400 DEBRID_ERROR` (provider unreachable...).

`PUT` may change only the username or only the password; omitted secrets are kept. Passwords and
session tokens are never returned.

Device flow: show `user_code` + a link to `verification_url`, then call `poll` every ~5 s until
`activated` (an account is created) or `expired`.

## Notifications

| Method | Path | Body | Response |
|---|---|---|---|
| POST | `/api/notifications/test` | `NotificationTarget` | `204` or `400 {error}` with the delivery error |

## Integrations

| Method | Path | Body | Response |
|---|---|---|---|
| POST | `/api/settings/api-key` | – | `{ api_key }` — generates a new API key (the old one stops working) |
| POST | `/api/integrations/telegram/test` | `{ token?: string }` | `{ username }` of the bot (`getMe`); empty/missing `token` = the saved one. `400 INVALID_TOKEN` (malformed), `400 TELEGRAM_ERROR` with Telegram's message (e.g. `Unauthorized`) |

Third-party tools can authenticate with the `X-Api-Key: <key>` header instead of the session cookie.

## Backup

A backup is a JSON document with the settings, the categories, the debrid accounts **including
their credentials** and the unfinished packages (as the links to add again). Users, sessions, the
history and the statistics are not included. Session only.

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/api/backup` | – | `Backup` as an attachment, `rustload-backup-YYYY-MM-DD.json` |
| POST | `/api/backup/restore?queue=false` | `Backup` (max 32 MiB) | `RestoreSummary`. `400 INVALID_BACKUP` (not a Rustload backup, unsupported `format`, invalid settings) |

```ts
type Backup = {
  format: 1
  app: "rustload"
  version: string                 // Rustload version that wrote it
  created_at: number
  settings: Settings              // complete, secrets included
  categories: { id: number, name: string, path: string, extensions: string[], is_default: boolean }[]
  accounts: { provider: string, label: string, enabled: boolean, credentials: object }[]
                                  // credentials as stored, without the session `token` of
                                  // username/password accounts (renewed by logging in)
  queue: {
    name: string
    category: string | null       // category name, null = automatic
    options: {
      create_subfolder: boolean, extract: boolean, delete_archives: boolean, archive_password: string | null
      media?: MediaOptions        // packages with media links (yt-dlp) only
    }
    links: string[]               // "links" packages: files not downloaded yet
    source_kind: "links" | "magnet" | "torrent"
    source: string | null         // magnet URI or .torrent file name
    torrent: string | null        // .torrent content, base64
  }[]
}

type RestoreSummary = {
  settings: true                  // settings replaced
  categories: number              // categories updated or created
  accounts: number                // accounts added
  packages: number                // packages added again (queue=true)
  errors: string[]                // parts skipped, e.g. "account `x`: unknown provider `y`", "<package>: <error>"
}
```

Restore rules, applied in one database transaction (except the queue):

- **Settings** are replaced. The current API key is kept when the backup has none.
  `watch.category_id` is mapped to the restored category.
- **Categories** are matched by name (case-insensitive): matches keep their id and are updated, the
  others are created; categories missing from the backup are kept, after the restored ones. The
  backup's default category becomes the default (the current one stays if the backup has none).
- **Accounts** are added unless an account of the same provider already exists with the same
  `api_key`, else the same `username` (ignoring case), else the same OAuth `client_id`
  (Real-Debrid), else the same other credentials once their tokens (`token`, `access_token`,
  `refresh_token`, `expires_at`) are left out: renewed tokens do not duplicate an account.
- **Queue** (`queue=true`): every package is added again through the normal intake, **paused**.
  Magnets and torrents need a debrid account. Restoring twice adds the packages twice.

## Media sites (yt-dlp)

Links of known media sites (YouTube, Vimeo, Twitch, SoundCloud, X, TikTok, Instagram...) are
downloaded with yt-dlp instead of debrid accounts and hoster plugins. See
[configuration.md](configuration.md#media-sites-yt-dlp) for the behaviour.

```ts
type MediaOptions = {
  quality: "best" | "2160" | "1440" | "1080" | "720" | "480" | "360" | "audio"
  audio_format: "best" | "m4a" | "mp3" | "opus" | "flac"   // used when quality == "audio"
  container: "auto" | "mp4" | "mkv"
  subtitles: boolean
  subtitle_langs: string        // "en,fr", regular expressions such as "en.*", or "all"
  embed_metadata: boolean
  embed_thumbnail: boolean
  sponsorblock: "off" | "mark" | "remove"
}

type MediaMeta = {              // Download.media
  id: string | null             // id on the site, e.g. the YouTube video id
  title: string | null
  duration_secs: number | null
  thumbnail: string | null      // remote image URL
  uploader: string | null
  extractor: string | null      // yt-dlp extractor, e.g. "youtube"
}

type MediaInfo = {
  title: string | null
  uploader: string | null
  duration_secs: number | null
  thumbnail: string | null
  webpage_url: string | null
  extractor: string | null      // e.g. "youtube", "youtube:tab"
  is_live: boolean
  // count: total announced by the site (for a channel: the videos found in its tabs)
  playlist: { title: string | null, count: number } | null
  // at most 500; for a channel, the videos of its tabs (videos, shorts, lives) one after the other
  entries: { url: string, title: string | null, duration_secs: number | null }[]
  filesize_approx: number | null   // single video: estimated size of the default format selection
}

type MediaStatus = {
  enabled: boolean                                                  // settings.media.enabled
  ytdlp: { path: string, version: string | null, managed: boolean } | null   // null = not found
  ffmpeg: { path: string, version: string | null } | null
  updating: boolean                     // a yt-dlp installation/update is running
  ffmpeg_installable: boolean           // ffmpeg can be installed with POST /api/media/ffmpeg/install (Windows)
  installing_ffmpeg: boolean            // that installation is running
  last_update_check: number | null      // unix seconds
  latest_version: string | null         // latest official release seen (null until checked)
}
```

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/api/media/status` | – | `MediaStatus` |
| POST | `/api/media/install` | – | `MediaStatus` once the latest official yt-dlp is installed in `<config>/bin` (session only). `409 UPDATE_IN_PROGRESS`, `400 UNSUPPORTED_PLATFORM`, `502 INSTALL_FAILED` (network, checksum mismatch, binary does not run) |
| POST | `/api/media/ffmpeg/install` | – | `MediaStatus` once `ffmpeg.exe` and `ffprobe.exe` are installed in `<config>/bin` (session only, Windows; about 200 MB to download). Same errors as `/api/media/install` |
| POST | `/api/media/probe` | `{ url }` | `MediaInfo` (about 2–10 s). `400 MEDIA_UNAVAILABLE` with yt-dlp's message (private, removed, unsupported...) or a timeout (30 s, including the wait when 4 probes already run), `409 MEDIA_DISABLED`, `503 YTDLP_MISSING` |

`install` downloads the release asset for this platform from GitHub (`yt-dlp_musllinux[_aarch64]` on
musl Linux such as the Docker image, `yt-dlp_linux[_aarch64]` on glibc, `yt-dlp.exe` on Windows,
`yt-dlp_macos` on macOS), checks it against the release's `SHA2-256SUMS` and that it runs, then
replaces the managed copy atomically. `ffmpeg/install` downloads yt-dlp's recommended FFmpeg build
(`ffmpeg-master-latest-win64-gpl.zip` or `-winarm64-gpl.zip` from
[yt-dlp/FFmpeg-Builds](https://github.com/yt-dlp/FFmpeg-Builds)), checks it against the release's
`checksums.sha256`, and takes `ffmpeg.exe` and `ffprobe.exe` from its `bin` folder.

## Logs

```ts
type LogEntry = { id: number, ts_ms: number, level: "ERROR" | "WARN" | "INFO" | "DEBUG", target: string, message: string }
```

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/api/logs?limit=500&level=INFO` | – | `{ entries: LogEntry[] }` (oldest first, `level` = minimum level) |
| DELETE | `/api/logs` | – | `204` (clears the in-memory buffer) |
| GET | `/api/logs/file` | – | `text/plain` attachment, current log file |

## Settings

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/api/settings` | – | `Settings` |
| PUT | `/api/settings` | `Settings` | `Settings` (`400 INVALID_SETTINGS` with message) |

```ts
type Settings = {
  general: {
    language: string                // "en", "fr"... ; "" = follow the browser language
    theme: "dark" | "light" | "system"
  }
  downloads: {
    max_parallel: number            // 1..20, default 3
    connections: number             // per download, 1..32, default 8
    min_split_size_mib: number      // don't split chunks smaller than this, 1..64, default 4
    speed_limit_enabled: boolean
    speed_limit_kib: number         // KiB/s, global
    max_retries: number             // 0..50, default 5
    retry_delay_secs: number        // default 30
    timeout_secs: number            // connect/read timeout, default 30
    user_agent: string
    file_exists: "rename" | "overwrite" | "skip"
    auto_start: boolean             // start new packages immediately
  }
  schedule: {
    enabled: boolean                // when true, downloads only run inside the window
    start: string                   // "HH:MM" local time (system time zone, TZ in Docker)
    end: string                     // "HH:MM"; end < start means the window crosses midnight
    days: boolean[]                 // 7 entries, Monday first
    speed_rules: SpeedRule[]        // at most 10, default []; apply whether `enabled` is on or not
  }
  storage: {
    create_subfolder: boolean       // default for new packages
    min_free_mib: number            // MiB, 0..1048576, default 1024; 0 disables the disk-space guard
  }
  postprocess: {
    extract: boolean
    delete_archives: boolean
    passwords: string[]             // tried in order when an archive is encrypted
    verify_checksums: boolean       // check files against .sfv/.md5/.sha* files of the package and
                                    // provider hashes before extraction, default true
  }
  debrid: {
    enabled: boolean                // use accounts for supported hosts
    delete_magnets: boolean         // remove magnets from the provider once downloaded
  }
  watch: {                          // "hot folder": files dropped here are imported automatically
    enabled: boolean
    dir: string                     // relative to the config dir or absolute, default "/watch"
    category_id: number | null      // null = automatic
  }
  clicknload: {
    enabled: boolean                // Click'n'Load v2 listener (RUSTLOAD_CNL_HOST / RUSTLOAD_CNL_PORT)
    paused: boolean                 // add received packages paused (default true)
  }
  notifications: {
    targets: NotificationTarget[]
  }
  hooks: {                          // shell commands run with RUSTLOAD_* environment variables
    package_finished: string        // "" = disabled
    package_failed: string
  }
  integrations: {
    api_key: string                 // read-only here; regenerate with POST /api/settings/api-key
    metrics: boolean                // serve Prometheus metrics on /metrics (default false)
  }
  telegram_bot: {                   // bot receiving links and commands (long polling, no public URL)
    enabled: boolean                // default false
    token: string                   // from @BotFather, "123456789:AAE..."; required when enabled
    allowed_chats: string[]         // chat ids (integers as strings, e.g. "123456789", "-1001234567890")
  }
  media: {                          // media sites through yt-dlp
    enabled: boolean                // default true
    ytdlp_path: string              // binary or its folder; "" (default) = managed copy, then PATH
    ffmpeg_path: string             // binary or its folder; "" (default) = <config>/bin, then PATH
    auto_update: boolean            // default true: daily update of the managed yt-dlp
    generic_pages: boolean          // default false: try yt-dlp on unrecognised web pages
    cookies_file: string            // Netscape cookies.txt, relative to the config dir or absolute; "" = none
    extra_args: string              // advanced, appended to every yt-dlp run (quotes group words, no shell);
                                    // options Rustload manages (-o, -P, --print, --exec...) are refused
    defaults: MediaOptions          // best / best / auto / no subtitles ("en") / metadata and cover embedded / SponsorBlock off
  }
}

// While a rule is active, the effective speed limit is the lowest of the global limit (if enabled)
// and of the active rules' limits. Same window rules as the schedule: equal start/end = whole day,
// end < start = crosses midnight (the part after midnight belongs to the day it started).
type SpeedRule = {
  enabled: boolean                  // default true
  start: string                     // "HH:MM" local time, default "08:00"
  end: string                       // "HH:MM", default "23:00"
  days: boolean[]                   // 7 entries, Monday first
  limit_kib: number                 // KiB/s, >= 1, default 1024
}

type NotificationEvent =
  | "package_finished" | "package_failed" | "queue_finished" | "extract_failed"
  | "checksum_failed"               // a file did not match its checksum (sent for every mismatch)
  | "disk_low"                      // free space fell below storage.min_free_mib (once per episode)

type NotificationTarget = {
  id: string                        // client-generated, stable
  name: string
  kind: "webhook" | "discord" | "telegram" | "ntfy" | "gotify"
  enabled: boolean
  url: string                       // webhook URL / Discord webhook / ntfy topic URL / Gotify server URL
  token: string                     // Telegram bot token / Gotify app token / ntfy access token ("" if none)
  chat_id: string                   // Telegram only
  events: NotificationEvent[]
}
```

Watch folder formats: `.torrent`, `.magnet` / `.txt` / `.links` (one link or magnet per line). Imported
files are moved to `<dir>/.imported/` (or `.failed/`).

## Live events — `GET /api/events` (Server-Sent Events)

| event | data |
|---|---|
| `stats` (every second) | `{ speed, active, queued, paused, schedule_open, speed_limit, speed_rule_active, queue_remaining, eta, disk_free, disk_total, disk_low }` — `speed_limit` is the **effective** limit in bytes/s (global limit and active speed rules) or `0`, `speed_rule_active` tells whether a speed rule currently applies, `eta` seconds or `null`, `disk_low` as in `System` |
| `progress` (every second, only when something is active) | `{ downloads: [{ id, package_id, status, downloaded, size, speed, eta, connections, map, heads }], packages: [{ id, status, debrid, extract_progress, verify_progress }] }` — `map`/`heads`: see *Segment map*; `packages[].status` is `"debrid"`, `"verifying"` or `"extracting"` |
| `queue` | `{}` — queue structure changed, refetch `/api/packages` |
| `history` | `{}` — history changed |
| `log` | `LogEntry` |
| `notice` | `{ level: "success" \| "info" \| "warning" \| "error", key: string, params: Record<string, string>, message: string }` — toast-worthy events; translate with `key`/`params`, `message` is the English fallback. Keys: `package_finished` {name}, `package_failed` {name, error}, `extract_failed` {name, error}, `queue_finished`, `download_failed` {name, error}, `account_error` {label, error}, `watch_imported` {file, count}, `clicknload_added` {name, count}, `debrid_ready` {name}, `checksum_failed` {name, file, algo} (`name` = package; level `warning` when the file is downloaded again, `error` when it fails), `disk_low` {free} (bytes, decimal string; once each time the guard trips), `telegram_added` {name, count} (name: package names joined with ", ", count: links or files received), `media_updated` {version} (the managed yt-dlp was updated) |
| `resync` | `{}` — client missed events, refetch everything |
