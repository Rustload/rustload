# Architecture

Rustload is a single Rust binary (Tokio + axum + SQLite) that embeds a Svelte web interface. This
page explains how the pieces fit together, for contributors.

```
                    ┌────────────────────────── rustload ───────────────────────────┐
 browser ── HTTP ──►│ api/      REST + SSE (/api/events)        web.rs  embedded UI   │
                    │   │                                                             │
                    │   ▼                                                             │
                    │ engine/   scheduler ─► task ─► resolve ─► transfer ─► file      │
                    │   │          │          │        │                              │
                    │   │          │          │        ├─ debrid/   (accounts)        │
                    │   │          │          │        └─ hosters/  (built-in plugins)│
                    │   │          │          └─ media    (media sites through yt-dlp)│
                    │   │          ├─ remote   (magnets processed by debrid)          │
                    │   │          ├─ checker  (names, sizes, dead links)             │
                    │   │          ├─ verify   (checksums, checksum.rs)               │
                    │   │          └─ postprocess ─► extract/ (7-Zip, ZIP)            │
                    │   ▼                                                             │
                    │ db/  SQLite (sqlx)    settings.rs  live settings (watch channel)│
                    │ integrations/  watch folder · Click'n'Load · notify · hooks     │
                    └─────────────────────────────────────────────────────────────────┘
```

## Source layout

| Path | Role |
|---|---|
| `src/lib.rs` | `Server::start`: privilege drop, logging, database, engine, HTTP server; `Server::stop`; the `Handle` the desktop app drives it with |
| `src/main.rs` | The server binary: configuration from the environment, stop on Ctrl+C or SIGTERM |
| `src/config.rs` | Environment variables |
| `src/settings.rs` | User settings: JSON document in the database, validated, broadcast through a `watch` channel |
| `src/db/` | One module per table; plain SQL with `sqlx` |
| `src/models.rs` | Database rows and the enums stored in them (statuses, stages) |
| `src/engine/` | The download engine (below) |
| `src/debrid/` | Debrid providers behind the `DebridService` trait, and `DebridManager` (account selection) |
| `src/hosters/` | Built-in hoster plugins behind the `Hoster` trait |
| `src/media/` | Media sites through yt-dlp: site detection, options, child process and output parsing, managed binary |
| `src/extract/` | Archive detection (volume grouping) and extraction |
| `src/checksum.rs` | Hash algorithms, checksum file parsing (`.sfv`, `.md5`, `.sha*`), streaming file hashing |
| `src/integrations/` | Watch folder, Click'n'Load, notifications, hooks, Telegram bot |
| `src/api/` | HTTP handlers, authentication, JSON views |
| `src/events.rs` | Event bus feeding Server-Sent Events |
| `src/stats.rs` | Download statistics: recorded as files finish (each file and package once, `counted` columns), aggregated for `/api/stats` |
| `src/backup.rs` | Backup export, validation and restore (settings, categories, accounts, queue) |
| `src/diagnostics.rs` | Installation checks ("doctor"): one function per check, listed in `diagnostics::run` |
| `src/logging.rs` | stdout + rotating file + in-memory buffer streamed to the UI |
| `src/privileges.rs` | `PUID`/`PGID`/`UMASK` handling |
| `web/` | Svelte 5 + Tailwind interface (see `web/DESIGN.md`) |
| `desktop/` | The desktop app (Tauri), a Cargo workspace of its own (below) |

## Data model

A **package** groups files added together (a list of links, or the content of one magnet or
torrent). It owns the destination folder and the post-processing options and moves through stages:

```
remote ──► download ──► verify ──► extract ──► done (history)
(magnet)      │   ▲          │
              │   └──────────┘ corrupted files are downloaded again once
              └── failed files stay in the queue until retried or removed
```

A package leaves `verify` for `extract` only when every file is done: files rejected by a
verification that was interrupted (restart) send it back to `download`, and an unexpected error
keeps it in `verify` for another attempt a minute later.

A **download** is one file. Its status is `queued`, `resolving`, `downloading`, `paused`,
`completed`, `failed` or `skipped`. The UI status of a package is derived from its stage and files
(`PackageRow::status`).

## The engine

`Engine::run` is the scheduler. It wakes up on every change (and every two seconds) and:

1. stops everything if the queue is paused, the schedule window is closed or the disk is almost
   full (`disk_guard.rs`);
2. starts queued files until `max_parallel` transfers run (`task::run` per file);
3. starts a poller for each package waiting on a debrid service (`remote.rs`);
4. starts one checksum verification (`verify.rs`) and one extraction (`postprocess.rs`) at a time;
5. runs the availability checker on new links (`checker.rs`).

Running work is tracked in memory by `LiveRegistry`: byte counters (atomics), speeds sampled once
per second, and a cancellation token with a stop reason (pause, requeue, remove). A one-second
ticker publishes `stats` and `progress` events.

### A download, step by step (`engine/task.rs`)

Media links (and web pages with `media.generic_pages`) take another path, see below.

1. **Resolve** (`resolve.rs`): the account pinned to the file (files of a debrid transfer), then
   debrid accounts supporting the host in priority order, then a built-in hoster plugin, then the
   link itself.
2. **Probe**: the first request (`Range: bytes=0-`) tells whether the server supports ranges, the
   size and the file name — and its body is kept to start downloading immediately.
3. **Folder**: the package folder is resolved once (category, automatic category, sub-folder).
4. **Transfer** into `<name>.<id>.rlpart`, with the resume state saved every 10 s.
5. **Finalize**: rename according to the file-exists policy, then let `lifecycle.rs` move the
   package forward (extraction, history, notifications, hooks).

Errors are classified: *transient* errors are retried with exponential backoff, *fatal* ones fail
the file, *stopped* puts it back as paused or queued.

### Segmented transfer (`engine/transfer.rs`)

The file is pre-allocated and every connection writes at its own offset (`pwrite`). Work is split
dynamically: a connection that becomes free takes the second half of the largest remaining
segment. A segment's end can therefore shrink while it downloads; the owner checks the bound before
each write, and splits always happen at least `min_split` bytes past the data being written (writes
are buffered by at most 256 KiB, which is always smaller than `min_split`).

When a server answers 429/503 to an extra connection while others are streaming, that connection
hands its segment back and stops: parallelism converges to what the server accepts, and the limit
is remembered per host for an hour. The resume state is the list of remaining byte ranges, written
after an `fsync` of the data.

Encrypted hosters (MEGA) provide a `Decryptor`: CTR mode is seekable, so each buffer is decrypted
at its offset before being written, keeping multi-connection downloads and resume.

The live registry keeps a reference to the segments of a running transfer. The one-second ticker
reads their bounds (remaining ranges, write position of each connection) and turns them into the
100-character segment map of the `progress` event; the transfer itself does no extra work.

### Link grabber (`engine/grabber/`)

`POST /api/links/check` previews links before anything is added. Each link goes through the same
decisions as a real download: `handler.rs` picks who downloads it in the order `resolve.rs` uses
(debrid account, hoster plugin, the link itself) and is shared with the availability checker, whose
probe (`checker::probe_with`) gives the name, size and status. Folder links are listed by their
plugin, duplicates are looked up by URL (indexed) and magnet info-hash, and direct links answering
with HTML can be scanned for downloadable links (`grabber/page.rs`, a tolerant attribute and text
scanner). The interface then adds what the user keeps through `POST /api/packages/batch`, i.e. the
regular intake, which expands folder links as usual.
### Media downloads (`engine/media.rs`, `media/`)

Links recognised by `media::sites` (or already handled by yt-dlp) skip resolution: yt-dlp runs as a
child process in a private work folder next to the destination (`.rustload-media-<id>`). Rustload
never parses yt-dlp's human-readable output: `--print` and `--progress-template` produce tagged,
machine-readable lines (`[rustload:info]`, `[rustload:progress]`, `[rustload:file]`...), and only
`ERROR:` lines are read to explain failures, classified as permanent or retryable for the usual
retry policy. Video and audio streams are downloaded one after the other; `ProgressTracker` adds
them up into one monotonic counter fed to `LiveDownload`, so the dashboard needs nothing special.
The finished file is moved into place with the regular file-exists policy and name sanitising.

The official yt-dlp builds are PyInstaller bundles whose bootloader starts the real process as a
child: stopping a download kills the whole tree (process group on Unix, `taskkill /T` on Windows),
and so does dropping the future of a run before the process exited (`process::Tree`'s `Drop`, for
abandoned HTTP requests and outer time limits). On Unix, runs get `TMPDIR=<config>/tmp`, emptied at
startup, because the bundles unpack themselves there.

Link checks probe media links with `yt-dlp -J` (title, duration, size estimate), at most four at a
time (a semaphore in `MediaTools`, whose wait counts in the probe's time limit); playlists are
listed when added (`--flat-playlist`) and become one download per entry. `media::tools` finds
yt-dlp and ffmpeg (cached detection), installs the official release with checksum verification,
and `engine::media::maintain` checks for updates daily without blocking the scheduler.

## Adding a debrid provider

1. Create `src/debrid/<provider>.rs` with a `pub const ID`, a `pub const INFO: ProviderInfo` and a
   type implementing `DebridService` (account status, supported hosts, unlock, magnets/torrents,
   transfer status/files, delete). Map provider errors to `DebridError` variants carefully: they
   drive the fallback between accounts (`Quota`/`Temporary` → next account, `LinkDown` → give up).
2. Register it in `src/debrid/mod.rs` (`providers`, `build`, `api_key_credentials`, and the device
   flow functions if the provider supports one).
3. If transfer files are direct links, mark them with `mark_direct` so that `unlock` passes them
   through.

Premium accounts of file hosters are providers too (`kind: Hoster`, `magnets: false`):
`supported_hosts` returns their fixed domain list (`HostMatcher::from_domains`) and the magnet
methods keep their default implementation. Username/password providers (`auth: Credentials`) keep
their session token in the stored credentials through `debrid/session.rs` and log in again when it
expires (`take_updated_credentials` persists the new token).

## Adding a hoster plugin

Implement `Hoster` in `src/hosters/<name>.rs`: `matches` (cheap URL test), `resolve` (direct URL,
optional headers, file name, size, decryptor) and optionally `expand` for folder links. Register it
in `Hosters::default`. Plugins never touch files; the engine downloads what they return.

## Web interface

`web/` is a Svelte 5 single-page application using hash routes and relative URLs (so it works under
any reverse-proxy sub-path). It is built into `web/dist` and embedded in the binary by `rust-embed`
(read from disk in debug builds). One `EventSource` connection keeps the stores up to date;
everything else is plain `fetch` through `src/lib/api`.

## Desktop app

`desktop/` wraps the same engine and interface in a Tauri app. It is a separate Cargo workspace with
its own lock file, so the server, its Docker image and its checks never build it; it depends on the
`rustload` library by path. On first start `desktop/ui/index.html` (the only page bundled in the
app, the one allowed to call its commands) asks how it is used, and the choice is kept in
`desktop.json` in the app's data folder:

- **On this computer** (`backend::Backend::Local`): `Config::desktop` starts the server on
  `127.0.0.1` (port 8800 unless taken), with an account named after the user and a random password.
  The window opens the interface through `Handle::login_url`, a single-use `/api/auth/handoff`
  address, so there is no login screen.
- **Gateway to a server** (`Backend::Remote`): no engine runs; the window shows the server's own
  interface (its login screen once, then its session cookie), and the app follows
  `/api/events` with the server's API key.

Either way `tray.rs` turns the `stats` and `notice` events into the tray tooltip, the pause item and
system notifications, and magnet links handed over by the system (command line on Windows and Linux,
an event on macOS) open the add dialog at `/?add=…`. A second launch passes its arguments to the
running instance (single-instance plugin). `updater.rs` checks the release's `latest.json`
(written by `.github/scripts/update_manifest.py`), downloads signed updates and installs them in a
quiet moment. `portable.rs` turns on the portable mode when a `data` folder sits next to the
program: the app's data, its webviews' data and the remembered window size (kept in logical pixels
in `desktop.json`, so a change of display scale does not shrink the window) then live there. The interface window loads an `http://` address, so it
has no access to the app's commands.
