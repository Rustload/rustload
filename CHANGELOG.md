# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Desktop app for Windows, macOS and Linux: Rustload in a window of its own that opens without a
  login, with an icon in the notification area that shows the speed, pauses everything and keeps
  downloading once the window is closed. It can start with the computer, open magnet links and
  turns finished and failed packages into system notifications. On first start it asks whether to
  download on this computer or to drive a Rustload server (gateway mode, with the server's API key).
  It updates itself with signed updates, installed when nothing is downloading. On Windows a
  portable copy (`…_x64-portable.zip`) runs without installing and keeps its data in its own `data`
  folder. The installers are not signed yet.
- On Windows, ffmpeg (yt-dlp's recommended build) and 7-Zip install in one click from *Settings ›
  Video sites* and *Settings › Extraction*, checked against their checksums and used without a
  restart.

### Changed

- Releases carry the browser extension as `rustload-extension.zip`, always at the same address
  (`releases/latest/download/rustload-extension.zip`), and *Get the extension* in the settings
  opens its install guide.

## [1.0.0] - 2026-10-16

First public version.

### Downloads

- Multi-connection segmented downloads with dynamic splitting, resume after restart, automatic
  retries, per-host connection limit detection, global speed limit, a download time window and
  speed rules by time of day.
- Live segment map of every multi-connection transfer, with the position of each connection.
- Packages, queue with search, filters, bulk actions and reordering, history with search and
  redownload, statistics (volume per day, busiest hours, hosts, sources), live logs.
- Free-space guard: downloads wait instead of filling the disk and resume on their own.

### Adding links

- Link grabber: links are checked before they are added (name, size, availability, account or
  plugin that will download them, duplicates already in the queue or history), grouped into
  packages (archive sets, folders, playlists, similar names) and web pages can be scanned for
  downloadable links.
- Browser extension for Chrome, Edge, Brave and Firefox: context menus, a picker for all the links
  of a page that checks them as it opens, a popup with live speed, a paste box and *Download this
  page* when the page is one your accounts or sites handle.
- Bookmarklet, share target for the installed web app (send links from a phone's share sheet),
  Telegram bot (links, `.torrent` files, `/status`, `/queue`, `/pause`, `/resume`), watch folder,
  Click'n'Load and a REST API with an API key, including a batch endpoint.

### Hosts and accounts

- Debrid providers: AllDebrid, Real-Debrid, Premiumize.me, Debrid-Link, TorBox — links, magnets and
  `.torrent` files, account priority with fallback, "connect with a code" where available.
- Premium hoster accounts through official APIs: 1fichier, Rapidgator, Keep2Share (FileBoom,
  Tezfiles).
- Built-in hosters: MEGA (with on-the-fly decryption), Google Drive, Dropbox, MediaFire,
  Pixeldrain, Gofile, including folder links.
- Video and audio sites through yt-dlp: quality presets, audio only, playlists, subtitles,
  SponsorBlock, managed and self-updating yt-dlp binary; ffmpeg in the Docker image.
- M3U playlists are read to know what they are: HLS streams (`.m3u8`) are downloaded by yt-dlp as
  one video file named after their address, other M3U lists are added as the links they list.

### After downloading

- Checksum verification (SFV, MD5, SHA-1, SHA-256, SHA-512 and hashes reported by hosts), with one
  automatic redownload of corrupted files.
- Automatic extraction of RAR/7z/ZIP/tar and multi-volume archives with a password list.
- Categories with automatic selection by rule (regular expression on the package name, links and
  file names) or file extension, folder per package. A new installation starts with the download
  root only; folders outside it (mounted volumes) must exist, or their files fail with a clear error.
- Notifications (webhook, Discord, Telegram, ntfy, Gotify, browser), hooks on completion or failure.

### Operating

- Prometheus metrics, a public health endpoint, a health check page (folders, disk, permissions,
  tools, accounts, connectivity), backup and restore of settings, categories, accounts and queue.
- Web interface in English and French, dark and light themes, phone layout, installable as an app,
  command palette and keyboard shortcuts, setup checklist on first start.
- Docker image for `linux/amd64` and `linux/arm64` with `PUID`/`PGID`/`UMASK` support.
- Ready-to-run binaries for Linux (static, x86-64 and ARM64), Windows and macOS, published with
  their checksums and a build provenance attestation.

[Unreleased]: https://github.com/rustload/rustload/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/rustload/rustload/releases/tag/v1.0.0
