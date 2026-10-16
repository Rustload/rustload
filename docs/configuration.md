# Configuration

Rustload has two kinds of configuration:

- **Environment variables**, read at startup: paths, ports, user/group, bootstrap account.
- **Settings**, edited live in the web interface (*Settings*) and stored in the database. They
  apply immediately, without restart.

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `RUSTLOAD_CONFIG_DIR` | `./config` (`/config` in Docker) | Database (`rustload.db`), logs, kept `.torrent` files |
| `RUSTLOAD_DOWNLOAD_DIR` | `./downloads` (`/downloads` in Docker) | Download root; relative category paths live here |
| `RUSTLOAD_HOST` | `0.0.0.0` | Listening address of the web UI and API |
| `RUSTLOAD_PORT` | `8800` | Web UI and API port |
| `RUSTLOAD_CNL_HOST` | `127.0.0.1` (`0.0.0.0` in Docker) | Listening address of Click'n'Load |
| `RUSTLOAD_CNL_PORT` | `9666` | Click'n'Load port (only used when enabled in the settings) |
| `RUSTLOAD_AUTH` | `true` | `false` disables the login screen — only behind an authenticating reverse proxy |
| `RUSTLOAD_ADMIN_USER` / `RUSTLOAD_ADMIN_PASSWORD` | – | Creates the administrator on first start instead of the setup screen |
| `RUSTLOAD_SECURE_COOKIES` | `false` | Mark the session cookie `Secure` (set it when served over HTTPS) |
| `RUSTLOAD_LOG` | `info` | Log filter, e.g. `debug` or `info,rustload::engine=debug` (`RUST_LOG` also works) |
| `PUID` / `PGID` | – | User and group owning every created file (when started as root, as in Docker) |
| `UMASK` | – | Permission mask of created files: `022` → 644/755, `002` → 664/775. Unset, the process keeps its own (`022` in the Docker image) |
| `TZ` | `UTC` | Time zone used by the download schedule |

### Files and permissions

The Docker image starts as root only to apply `PUID`/`PGID`: it takes ownership of `/config` and
of the `/downloads` folder itself, then drops privileges. Every file and folder is then created as
`PUID:PGID` with the `UMASK` permissions — including extracted files, whatever permissions were
stored in the archive. You can also run the container directly as a user (`user: "1000:1000"`);
`PUID`/`PGID` are then ignored.

Partial downloads are written next to their final location as `<name>.<id>.rlpart` and renamed
when complete, so media servers never see incomplete files under their real name. Media-site
downloads use a hidden `.rustload-media-<id>/` folder instead (see below).

## Folders and categories

*Settings › Folders* lists the **categories**. Each one has a folder (relative to the download root,
or absolute such as `/media/movies` if you mounted it in the container) and an optional list of
file extensions.

When adding links you pick a category, or **Automatic**: Rustload looks at the file names of the
package and picks the category matching most of them (e.g. `mkv,mp4` → *Videos*), otherwise the
default category. Four categories are created on first start: *Downloads* (default, the root),
*Videos*, *Series* and *Music*.

**Create a folder for each package** puts the files of a package in `<category>/<package name>/`.
For torrents, the torrent's own folder structure is kept below it.

## Downloads

| Setting | Default | Notes |
|---|---|---|
| Parallel downloads | 3 | Files transferring at the same time |
| Connections per download | 8 | Upper bound; Rustload uses fewer if a server refuses more, and remembers it per host for an hour |
| Minimum split size | 4 MiB | A part is not split again below twice this size |
| Speed limit | off | Global, shared by every connection; see also the speed rules below |
| Retries / retry delay | 5 / 30 s | Transient errors only; the delay doubles at each attempt (max 1 h) |
| Timeout | 30 s | Connection and read timeout |
| If the file exists | rename | `rename` → `file (1).ext`, `overwrite`, or `skip` |
| Start automatically | on | When off, new packages are added paused |

### Speed rules

*Speed rules* lower the speed limit during time windows — "slow during the day, full speed at
night": each rule has a start and end time, days of the week and a limit (KiB/s). Up to 10 rules;
they use the same windows as the schedule (container time zone, end before start crosses
midnight, equal times cover the whole day) and apply even when the schedule itself is off. When
rules overlap, or a global speed limit is set, the lowest limit wins. Changes apply immediately,
and the limit switches by itself when the clock enters or leaves a window.

### Disk space

Downloads pause by themselves when the download folder runs out of space: below *Keep at
least* (1024 MiB by default, `0` disables the guard) running transfers go back to the queue —
they resume from where they stopped — and nothing new starts. A notification (`disk_low`) is sent
once, and downloads resume automatically when the free space is back above the minimum plus 10 %.
The free space is the one of the download root (`RUSTLOAD_DOWNLOAD_DIR`), checked every few
seconds.

### Schedule

With *Only download during a time window*, downloads run between the start and end times on the
selected days, using the container time zone (`TZ`). A window ending before it starts crosses
midnight (e.g. 23:00 → 07:00 belongs to the day it starts). Outside the window running downloads
are paused and resume automatically, from where they stopped.

## Checksum verification

When every file of a package is downloaded, and before extraction, Rustload checks the files
against the checksums it knows (*Settings › Extraction › Verify files with their checksums*, on by default):

- checksum files of the package: `.sfv` (CRC32), `.md5`, `.sha1`, `.sha256` and `.sha512`, in the
  `hash  name` / `hash *name` format of `md5sum`/`sha256sum`, the BSD format
  (`SHA256 (name) = hash`), or holding a single hash for the file of the same name (`movie.mkv.md5`);
- hashes published by the hoster or account that provided the file (Pixeldrain SHA-256, Rapidgator
  MD5, Keep2Share/FileBoom/Tezfiles MD5 of your own files).

When several are known for a file, the strongest algorithm is used. Verified files get a
"verified" mark. A file that does not match is deleted and downloaded again once; if it still does
not match, it fails with `checksum mismatch (<algo>)` and the package is reported as failed (retry
it from the queue). Every mismatch also sends the `checksum_failed` notification. A file that
cannot be read (deleted or moved meanwhile, permissions) fails with `cannot read the file to verify
its checksum: ...` instead of passing unchecked; retrying it downloads it again. The package only
moves on to extraction once every file is downloaded and verified, also after a verification was
interrupted by a restart.

## Extraction

When every file of a package is downloaded (and verified) and *Extract archives* is on, archives are extracted
next to them: RAR (including `part1.rar` and `.r00` volumes), 7z (`.7z.001`), ZIP (including
`.z01` and `.zip.001`), tarballs and split files (`.001`). Archives are extracted in a hidden
staging folder first and only moved into place once complete; symlinks inside archives are
dropped.

Passwords are tried in this order: none, the package password, then the global list
(*Settings › Extraction*). A password that works moves to the top of the list. Extraction needs
7-Zip for everything but plain ZIP files; the Docker image includes it.

## Accounts

*Settings › Accounts › Add account*: choose a provider, then paste an API key or — for AllDebrid,
Real-Debrid and TorBox — use **Connect with a code** and approve Rustload on the provider's
website.

| Provider | API key page | Connect with a code |
|---|---|---|
| AllDebrid | <https://alldebrid.com/apikeys/> | yes |
| Real-Debrid | <https://real-debrid.com/apitoken> | yes |
| Premiumize.me | <https://www.premiumize.me/account> | – |
| Debrid-Link | <https://debrid-link.com/webapp/apikey> | – |
| TorBox | <https://torbox.app/settings> | yes |

### Premium hoster accounts

Premium accounts of some file hosters can be added next to debrid accounts. They use the hosters'
official APIs and only handle links of their own site (all its domains).

| Hoster | Sign in with | Domains |
|---|---|---|
| 1fichier | API key, from <https://1fichier.com/console/params.pl> (premium accounts only) | 1fichier.com, alterupload.com, cjoint.net, desfichiers.com, dfichiers.com, megadl.fr, mesfichiers.org, piecejointe.net, pjointe.com, tenvoi.com, dl4free.com |
| Rapidgator | E-mail and password | rapidgator.net, rapidgator.asia, rg.to |
| Keep2Share | E-mail and password | k2s.cc, keep2share.cc, k2share.cc, keep2s.cc |
| FileBoom | E-mail and password | fboom.me, fileboom.me |
| Tezfiles | E-mail and password | tezfiles.com, publish2.me |

The premium status, expiry date and remaining traffic are shown when the hoster reports them.
Accounts with two-factor authentication cannot be used (Rapidgator), and when a site asks for a
captcha at login (Keep2Share, FileBoom, Tezfiles), log in once on its website from the same
network, then check the account again. 1fichier bans IP addresses that send too many failing
requests: Rustload spaces its calls and never checks 1fichier links before downloading them.

Nitroflare and DDownload are not supported: their official APIs either are not public or offer no
premium download.

### Order and fallback

Accounts are used from top to bottom (drag to reorder). For each link, the first account whose
provider supports the host unlocks it; if it fails because of a quota or a temporary problem, the
next one is tried and the failing account is put aside for a while. Links that no account supports
go to the built-in hoster plugins (MEGA, Google Drive, Dropbox, MediaFire, Pixeldrain, Gofile), then
are downloaded directly. Links of media sites (YouTube...) go to yt-dlp first, see below.

Magnets and `.torrent` files are sent to the first account able to handle them; the package shows
the remote progress until the files are ready, then downloads them like any other file.

> Some providers (notably AllDebrid and Real-Debrid) refuse requests coming from VPNs or data
> centres. Run Rustload on a residential connection, or route it accordingly.

## Media sites (yt-dlp)

Links of popular media sites are downloaded with [yt-dlp](https://github.com/yt-dlp/yt-dlp), before
debrid accounts and hoster plugins: YouTube (videos, Shorts, YouTube Music, playlists, channels),
Vimeo, Dailymotion, Twitch (videos and clips), SoundCloud, Bandcamp, X/Twitter, TikTok, Instagram,
Facebook, Reddit, Bilibili, ARTE, france.tv, Mixcloud, Rumble, Odysee, Streamable, Kick, Bluesky,
Threads, TED, Niconico, VK and OK.ru. With **Try yt-dlp on other web pages** (`generic_pages`), a
link to a web page that nothing else handles is also tried with yt-dlp before failing.

- **One link, one file.** The package is named after the video once it is probed, and the file is
  named `<title> [<id>].<ext>` (made safe for Windows/SMB shares; `<title>.<ext>` for streams and
  links that only have a name from their address). The automatic category picks *Videos* or
  *Music* from the expected extension.
- **Playlists, channels and albums** become their own package, named after the playlist, with one
  file per video (at most 500, in order). Their entries are not probed one by one: sizes appear
  when each video starts.
- **Live streams** are refused with a clear error: download the recording once the stream ended.
- The link check fills title, duration, thumbnail and an estimated size, and flags private, removed
  or geo-blocked videos before they start.
- Progress, speed, pause/resume and retries work as for other files. Partial streams are kept in a
  hidden `.rustload-media-<id>/` folder next to the destination; resuming continues them.
  Permanent errors (private or removed video, geo-blocking, sign-in or age gate, unsupported link)
  fail the file at once; network errors, server errors and throttling are retried.
- The **speed limit** in effect (the global limit, or a speed rule whose window is open) applies
  approximately: each yt-dlp download gets it divided by the number of running downloads when it
  starts (`--limit-rate`, not updated while it runs), and yt-dlp traffic is not counted by the
  limiter of regular downloads.
- **Without yt-dlp**, links of media sites that a debrid account or a hoster plugin supports are
  downloaded through them, as any other link (the log says so); the others fail with
  `yt-dlp is not installed`.
- At most **4 link checks** run yt-dlp at the same time (queue checks, the link grabber,
  `/api/media/probe`); the others wait for their turn within their time limit.

### Options

Each package has its own options (defaults in *Settings › Video sites*):

| Option | Default | Notes |
|---|---|---|
| Quality | best | `2160`…`360` caps the resolution (the smallest side, so vertical videos match their label; the closest larger one is used when nothing smaller exists), or `audio` |
| Audio format | best | With `audio`: `best` keeps the original stream, `m4a`/`mp3`/`opus`/`flac` convert |
| Container | auto | `mp4` prefers H.264/AAC streams; `mkv` accepts anything |
| Subtitles | off | Languages such as `en,fr`, `en.*` or `all` (subtitles written by the uploader, not automatic captions) |
| Embed metadata / cover | on / on | Title, artist, chapters and thumbnail written into the file |
| SponsorBlock | off | YouTube: `mark` adds chapters for sponsored segments, `remove` cuts them out |

### yt-dlp, ffmpeg and JavaScript

yt-dlp is looked up in this order: the path in *Settings › Video sites*, the copy managed by Rustload in
`<config>/bin/`, then `PATH`. **Install** (in *Settings › Video sites*) downloads the official build for
your system from GitHub, verifies its SHA-256 checksum and installs it in `<config>/bin/`; with
**Update automatically** (default) Rustload checks for a new release a few minutes after startup
and then daily. It updates the managed copy, and when the yt-dlp in use comes from the system or
the Docker image and is older than the latest release, it installs a managed copy that takes over
(about 35 MB in `<config>/bin/`). Sites change often: when a site stops working, update yt-dlp
first. A yt-dlp installed elsewhere is never modified, and nothing is installed automatically when
yt-dlp is missing or when its path is set in the settings.

**ffmpeg** combines separate video and audio streams (most YouTube qualities), converts audio and
embeds subtitles, metadata and covers. Without it, Rustload degrades gracefully and notes it in
the log: only single-file formats are downloaded (often 360p on YouTube, and some videos have none:
choose audio only), audio is kept in its original format, subtitles are saved as separate files,
and embedding and SponsorBlock are skipped.

YouTube also needs a **JavaScript runtime** for full format support: yt-dlp uses Deno, Node.js
(22 or newer) or QuickJS when one is installed.

On Linux and macOS, yt-dlp runs with `<config>/tmp` as its temporary folder (`TMPDIR`): the
official builds unpack themselves there at every start, which fails when `/tmp` is read-only or
mounted `noexec`. Rustload empties `<config>/tmp` when it starts (runs that were stopped leave
their unpacked copy behind): do not keep files there.

The Docker image includes yt-dlp, ffmpeg and QuickJS. The copy installed from the interface into
`/config/bin` takes precedence over the one in the image and is kept up to date.

### Sites that need a login

Some videos need an account (age-restricted, members-only, private Instagram content...) and
YouTube sometimes asks to "confirm you're not a bot". Export the cookies of a logged-in browser
session in the Netscape `cookies.txt` format (with a browser extension such as *Get cookies.txt
LOCALLY*), put the file in the config folder and set **Cookies file** (`cookies_file`, relative to
the config folder or absolute). Each yt-dlp run works on a private copy, so the file is never
modified. Treat it like a password.

### HLS streams and M3U lists

Links ending in `.m3u8` or `.m3u`, and links served as an M3U playlist
(`application/vnd.apple.mpegurl`, `audio/x-mpegurl`...), are read to know what they are, before
debrid accounts and hoster plugins see them:

- An **HLS stream** (a playlist with `#EXT-X-` tags: the master playlist of a streamed video, or
  one of its renditions) is one video downloaded by yt-dlp, whatever site serves it. The quality
  option picks among the renditions of a master playlist and the segments become a single file,
  named after the address since a stream has no title: `…/big-buck-bunny/hls/master.m3u8` gives
  `big-buck-bunny.mp4` (generic names such as `master`, `index`, `hls` or `720p` are skipped).
  Video sites must be enabled. Live streams are refused as on media sites. AES-128 encrypted
  streams work, DRM-protected ones do not. Without ffmpeg, renditions whose video and audio are
  separate cannot be combined: choose another quality or install ffmpeg.
- Any other playlist is a **list of links**: it is added as its entries (at most 1000) in one
  package named after the list, and each entry then follows its own route (debrid account, hoster
  plugin, media site, direct link). The link grabber shows the entries with their `#EXTINF`
  titles. Relative links are resolved against the playlist address; local paths and other
  protocols are skipped.
- Links recognised by their `Content-Type` only (no extension) are read by the link grabber and by
  the availability check of the queue. A stream that slipped through is still recognised when its
  download starts; a list found then is downloaded as the file it is.

Streaming servers often only answer the player of their own site: a link copied from the
browser's developer tools may get `403 Forbidden` without the site's `Referer` or cookies. A
`.m3u8` link Rustload cannot read is handed to yt-dlp as a stream anyway: add
`--referer https://the-site.example/` (or `--add-header "Origin:https://the-site.example"`) to
**Extra arguments**, set a **Cookies file** if needed, or add the page of the video instead when
yt-dlp supports the site.

### Advanced

**Extra arguments** (`extra_args`) are appended to every yt-dlp command, e.g.
`--proxy socks5://127.0.0.1:1080` or `--sub-format srt`. They are split like a shell would (quotes
group words) but no shell is involved. These options are refused, in full, with a value attached
(`--paths=/x`) or abbreviated (yt-dlp accepts any unambiguous beginning of an option name, such as
`--pat` for `--paths`, so every beginning of these names is refused):

- managed by Rustload: `--output`, `--paths`, `--print`, `--print-to-file`, `--progress-template`,
  `--newline`, `--no-progress`, `--progress`, `--quiet`, `--simulate`, `--skip-download`,
  `--no-download`, `--dump-json`, `--dump-single-json`, `--batch-file`, `--config-locations`
  (and its alias `--config-location`), `--load-info-json` (and `--load-info`), `--update`,
  `--update-to`, `--yes-playlist`, `--keep-video`, `--split-chapters`, `--list-formats`,
  `--list-subs`, `--list-thumbnails`, `--version`, `--help`, `--ffmpeg-location` (use *ffmpeg
  path*), `--` alone, and the short options `-o`, `-P`, `-O`, `-q`, `-s`, `-j`, `-J`, `-a`, `-U`,
  `-k`, `-F`, `-h` (also with a value attached, `-ofile`, or combined, `-qs`);
- running commands or loading code (use the hooks in *Settings › Integrations* to run commands):
  every option starting with `--exec` (`--exec`, `--exec-before-download`...), `--netrc-cmd`,
  `--use-postprocessor` and `--plugin-dirs`.

`--netrc` itself is accepted. Arguments saved by an older version that are refused now are
ignored (all of them, with a warning in the log at startup) until they are changed.

## Adding downloads

Everything you add goes through the **link grabber** first (the *Add* button, <kbd>N</kbd>, or
pasting links anywhere outside a text field): links are checked before anything starts — name, size,
availability, which account, plugin or yt-dlp will download them, and whether they are already in
the queue or the history. Multi-part archives, folders and playlists become their own packages,
files with similar names are grouped together (*Everything in one package* makes a single one).
Links that lead to a web page can be scanned for the downloadable links on the page; M3U lists are
expanded into their links and HLS streams (`.m3u8`) downloaded as videos (see [HLS streams and M3U
lists](#hls-streams-and-m3u-lists)). Dropping `.torrent` files anywhere on the page opens the
grabber with them.

Other ways in:

- **Browser extension** (Chrome, Edge, Brave, Firefox): right-click a link, a selection or a page,
  or pick links from the whole page. See [extension/README.md](../extension/README.md); it uses the
  API key.
- **Bookmarklet**: drag *Send to Rustload* from *Settings › Integrations* to the bookmarks bar.
  It opens Rustload with the selected text, or the current page, in the grabber
  (`http://<server>:8800/?add=<link>` works too).
- **Phone share sheet**: once Rustload is installed as an app (*Settings › Integrations*, or the
  browser's *Install* / *Add to Home Screen* menu), it appears as a share target. Browsers only
  allow installing over HTTPS (or `localhost`), so put Rustload behind a reverse proxy with a
  certificate to use it.
- The Telegram bot, the watch folder, Click'n'Load and the API, described below.

The interface is keyboard friendly: <kbd>Ctrl</kbd>+<kbd>K</kbd> (or <kbd>/</kbd>) opens the
command palette to search packages and run actions, <kbd>G</kbd> then a letter jumps to a page,
<kbd>?</kbd> lists the shortcuts. *Settings › General* also offers desktop notifications for
finished and failed packages while the tab is in the background (HTTPS only, like installing).

## Watch folder

*Settings › Integrations › Watch folder*: files dropped in the folder (`/watch` by default — mount
it as a volume) are imported automatically and then moved to `.imported/` (or `.failed/`).

- `.torrent` files are sent to the debrid service.
- `.magnet`, `.txt` and `.links` files contain one link or magnet per line. Lines starting with
  `;` or `#` are comments, and `[Name]` starts a new package named *Name*.

## Click'n'Load

Link protectors and some download sites offer a *Click'n'Load* button that sends links to a
download manager listening on `http://127.0.0.1:9666`. Enable it in *Settings › Integrations*.

The browser always contacts `127.0.0.1:9666` on the machine it runs on. The protocol has no
authentication, so Rustload only listens on the loopback interface by default, only accepts
requests whose `Host` is a loopback address, and adds received links **paused** (this can be
changed in the settings).

- Same machine, without Docker: nothing to do.
- Docker on the same machine: the image listens on all interfaces *of the container*; publish the
  port on the loopback interface only, `127.0.0.1:9666:9666`. **Never** publish it as `9666:9666`.
- Remote server: forward the port from the computer running the browser, for example
  `ssh -L 9666:localhost:9666 user@server`.

## Telegram bot

Send links, magnets and `.torrent` files to a Telegram bot, and check or pause the queue from your
phone. The bot polls Telegram, so Rustload does not need to be reachable from the internet.

1. In Telegram, talk to [@BotFather](https://t.me/BotFather), send `/newbot` and follow the steps.
   It gives you a token such as `123456789:AAE...`.
2. Paste the token in *Settings › Integrations › Telegram bot*, use **Test** to check it, enable
   the bot and save.
3. Send any message to your bot. It answers that the chat is not allowed yet and gives its id:
   add that id to the allowed chats and save. Group chats work too (their ids start with `-`).

Allowed chats can then:

- send text containing links or magnets: they are added like links pasted in the interface
  (packages start automatically unless *Start automatically* is off);
- send a `.torrent` file (up to 10 MiB);
- use `/status` (speed, active and queued files, remaining size and time, free disk space),
  `/queue` (the first 10 packages), `/pause`, `/resume` and `/help`.

Messages with links or torrents are added one at a time, in the order they were sent (listing a
playlist can take up to a minute), while commands are answered right away. Messages from other
chats are ignored, except for one answer every 10 minutes giving the chat id.
One bot token can only be used by one program at a time: do not reuse the token of a bot that
another application already polls (the notification target can use the same bot, though).

## Notifications

*Settings › Notifications* sends a message when a package finishes, fails, when extraction fails,
when a file does not match its checksum, when the disk is almost full or when the whole queue is
done:

| Kind | URL field | Token field |
|---|---|---|
| Webhook | Any URL; receives a JSON `POST` (`event`, `title`, `message`, `package`, `timestamp`) | – |
| Discord | Channel webhook URL | – |
| Telegram | – | Bot token, plus the chat id |
| ntfy | Topic URL, e.g. `https://ntfy.sh/my-topic` | Access token (optional) |
| Gotify | Server URL | Application token |

## Hooks

*Settings › Integrations › Hooks* runs a shell command (`sh -c`) when a package finishes or fails.
Details are passed as environment variables:

| Variable | Content |
|---|---|
| `RUSTLOAD_EVENT` | `package_finished`, `package_failed` or `extract_failed` |
| `RUSTLOAD_PACKAGE_ID` | Package id |
| `RUSTLOAD_PACKAGE_NAME` | Package name |
| `RUSTLOAD_PACKAGE_DIR` | Destination folder |
| `RUSTLOAD_PACKAGE_STATUS` | `completed` or `failed` |
| `RUSTLOAD_PACKAGE_SIZE` | Total size in bytes |
| `RUSTLOAD_FILES` | Downloaded files, one path per line |

Example: `curl -s -X POST http://jellyfin:8096/Library/Refresh -H "X-Emby-Token: …"`.

## API and integrations

The REST API is documented in [API.md](API.md). Tools authenticate with the `X-Api-Key` header; the
key is shown (and can be regenerated) in *Settings › Integrations*. It can add and manage downloads
and read the queue, history and logs, but not change settings, accounts or notifications, which
require a logged-in user.

```sh
curl -H "X-Api-Key: $KEY" -H "Content-Type: application/json" \
     -d '{"links": "https://example.com/file.zip"}' http://localhost:8800/api/packages
```

## Monitoring

**Diagnostics**: *Settings* runs a series of checks — folders writable, disk space, 7-Zip, video
site tools (yt-dlp and ffmpeg), file ownership (`PUID`/`PGID`), time zone, debrid accounts,
authentication, internet access and system clock — and explains how to fix what is wrong. Include
its result when reporting a problem.

**Health check**: `GET /api/health` answers `200 {"status":"ok"}` without authentication while the
database answers (`503` otherwise). The Docker image uses it as its `HEALTHCHECK`; uptime monitors
can poll it too.

**Prometheus**: enable *Settings › Integrations › Prometheus metrics* to serve `/metrics` (speed,
queue, disk space, completed and failed files, debrid account traffic and expiry; accounts are
labelled with their provider and id, never their name, which often holds an e-mail address).
Scrapers authenticate with the API key as a bearer token:

```yaml
scrape_configs:
  - job_name: rustload
    metrics_path: /metrics
    authorization:
      credentials: "<API key from Settings › Integrations>"   # sent as "Bearer <key>"
    static_configs:
      - targets: ["rustload:8800"]
```

The metric list is in [API.md](API.md#monitoring--get-metrics).

## Reverse proxy

The interface uses relative URLs and hash-based routes, so it works at the root of a domain or
under a sub-path. Live updates use Server-Sent Events (`/api/events`): disable response buffering
for that path.

```nginx
location /rustload/ {
    proxy_pass http://127.0.0.1:8800/;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_buffering off;          # live updates
    client_max_body_size 32m;     # .torrent uploads
}
```

When the proxy serves HTTPS, set `RUSTLOAD_SECURE_COOKIES=true`. If the proxy already
authenticates users, you can set `RUSTLOAD_AUTH=false`.

## Backups

*Settings* can download a JSON backup of the settings, categories, debrid accounts and the
unfinished queue. It contains your API keys and passwords: store it safely. Restoring it replaces
the settings, merges categories (by name) and accounts (an account already present — same provider
and same API key, login or Real-Debrid authorisation — is not duplicated, even if its session
tokens changed since), and can add the saved queue again, paused. Users, the history and the
statistics are not part of it.

For a complete copy, back up the config folder: `rustload.db` (SQLite, safe to copy while
Rustload is stopped) holds the queue, history, statistics, settings and accounts.
