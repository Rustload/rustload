# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [1.0.0] - Unreleased

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
  of a page, a popup with live speed and a paste box.
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
- Categories with automatic selection by file extension, folder per package.
- Notifications (webhook, Discord, Telegram, ntfy, Gotify, browser), hooks on completion or failure.

### Operating

- Prometheus metrics, a public health endpoint, a health check page (folders, disk, permissions,
  tools, accounts, connectivity), backup and restore of settings, categories, accounts and queue.
- Web interface in English and French, dark and light themes, phone layout, installable as an app,
  command palette and keyboard shortcuts, setup checklist on first start.
- Docker image for `linux/amd64` and `linux/arm64` with `PUID`/`PGID`/`UMASK` support.
