# Rustload browser extension

Send links, magnets and whole pages from your browser to your self-hosted
[Rustload](../README.md), and watch transfers live from the toolbar.

One codebase for **Chrome, Edge, Brave** (Manifest V3, Chromium 121+) and **Firefox 128+**.
Plain ES modules, HTML and CSS: no build step, no dependencies.

## What it does

- **Toolbar popup**: the total download speed as a live readout with the last minute of
  activity, what is in progress (one row per transfer that fills up as it goes), a paste box for
  links and magnets, *Pause all* / *Resume all*, and a link to open Rustload. Updates come live
  from the server (Server-Sent Events); if a proxy blocks streaming, the popup falls back to
  refreshing every 2 seconds.
- **Right-click menus**
  - *Send link to Rustload* on any link (http, https or magnet).
  - *Send selected links to Rustload*: the links inside the selection, plus URLs written in
    the selected text.
  - *Send all links on this page…*: opens a picker with every link of the page, pre-selects
    the ones that look like downloads (archives, videos, known hosters and media sites such as
    YouTube, magnets), and lets you filter by name, extension or host before sending.
  - *Send this page to Rustload*: sends the page address itself (useful for media pages).
- **Badge**: the number of unfinished downloads, refreshed every minute and after each send.
  It is neutral grey, and turns orange only while something is actually downloading. `+3`
  briefly confirms a send; `!` means the server can't be reached (hover for the reason).
- **Magnet links in Firefox**: the extension registers itself as a handler for `magnet:` links.
  The first time you click a magnet link, Firefox asks which application to use: choose
  *Rustload*.
- English and French, light and dark (follows the system theme).

## Install

### Chrome, Edge, Brave (unpacked)

1. Open `chrome://extensions` (Edge: `edge://extensions`, Brave: `brave://extensions`).
2. Turn on **Developer mode**.
3. Click **Load unpacked** and select this `extension` folder (the one containing
   `manifest.json`).
4. Pin Rustload to the toolbar from the puzzle-piece menu.

The settings page opens by itself after installation.

### Firefox

For development or a quick try:

1. Open `about:debugging#/runtime/this-firefox`.
2. Click **Load Temporary Add-on…** and select `extension/manifest.json`.

A temporary add-on stays until Firefox restarts. For a permanent install, the add-on has to be
signed by Mozilla (submit the zip below to [addons.mozilla.org](https://addons.mozilla.org),
listed or self-distributed), or use Firefox Developer Edition / Nightly with
`xpinstall.signatures.required` set to `false`.

## Configuration

Open the extension's settings (gear icon in the popup, or right-click the toolbar icon →
*Options*):

| Setting | |
|---|---|
| **Server address** | The address you open Rustload at, with its port, e.g. `http://nas:8800`. `nas:8800` is understood as `http://nas:8800`; a reverse-proxy path such as `https://home.example/rustload` works too. |
| **API key** | In Rustload: **Settings → Integrations → API key**. Regenerating the key there disconnects the extension until you paste the new one. |
| **Test connection** | Calls `GET /api/system` and shows the Rustload version, or exactly what is wrong: server unreachable, no answer in time, wrong or missing API key, or an address that answers but isn't Rustload. |
| **Add paused** | Packages sent from the browser wait in the queue until you start them. The popup has its own *Paused* checkbox, preset from this setting. |
| **Category** | Destination folder for what you send; *Automatic* lets Rustload choose from the file extensions. The list is loaded from the server. |
| **Confirm with a notification** | A short system notification after a send from a menu. Errors are always notified. |

When you click **Save** (or **Test connection**), the browser asks once for permission to
access that server address. Nothing works without it, and no other site is ever requested.

## Permissions

| Permission | Why |
|---|---|
| Access to your server (optional host permission, asked at runtime for that one address) | Send links and read the queue through the Rustload API. The manifest declares `optional_host_permissions: ["*://*/*"]` only so that any address can be *requested*; nothing is granted until you approve your server. |
| `activeTab` + `scripting` | Read the links of the current page, only when you choose *Send all links on this page…*, *Page links* in the popup, or *Send selected links*. No content script runs on pages otherwise. |
| `contextMenus` | The right-click entries. |
| `storage` | Keep the settings in `storage.local` (on this device, never synced) and hand the page links to the picker window (`storage.session`, memory only). |
| `alarms` | Refresh the badge once a minute. |
| `notifications` | Confirm sends and report errors. |

Security notes: the API key is stored in the extension's local storage and only ever sent to
your server, in the `X-Api-Key` header (requests never carry cookies). It is never logged. Every
page uses a strict Content Security Policy (no inline or remote code), and the server address
is validated (http/https only, no credentials in the URL).

## Package a zip

The zip must contain `manifest.json` at its root. With Mozilla's tool (works for both the
Chrome Web Store and addons.mozilla.org):

```sh
npx web-ext build --source-dir extension --artifacts-dir dist --ignore-files README.md
# → dist/rustload-1.0.0.zip
```

or by hand on Linux/macOS:

```sh
cd extension && zip -r -X ../rustload-extension-1.0.0.zip . -x README.md
```

(Avoid Windows PowerShell 5's `Compress-Archive`: it writes `\` in paths, which Firefox rejects.)
`npx web-ext lint --source-dir extension` checks the extension against Firefox's rules; its only
expected warning is that Firefox ignores `background.service_worker` (it uses
`background.scripts`, declared next to it).

## Development

```
manifest.json          MV3 manifest (Chromium service worker + Firefox background scripts)
background.js          context menus, badge refresh, page-links picker window
popup/                 toolbar popup
options/               settings page
picker/                "Send all links on this page…" window
handler/               Firefox magnet: protocol handler page
lib/                   shared modules: API client, settings, i18n, formatting, links, badge…
styles/base.css        Rustload design tokens and building blocks (see web/DESIGN.md)
fonts/                 Archivo variable font (SIL Open Font License, fonts/OFL.txt)
icons/                 toolbar and store icons, rendered from the brand files in docs/brand
_locales/{en,fr}/      messages
```

- **Design**: the extension follows `web/DESIGN.md`: same tokens as `web/src/app.css`
  (light "aluminium", dark "steel"), Archivo with condensed figures (`.num`, `.readout`), the
  signal colour (oxide / ember, see `docs/brand`) only for live transfers and the single
  primary action, rows rather than cards, 1 px borders, 6 px controls and 10 px panels. Icons are inline SVG from [Lucide](https://lucide.dev) (ISC
  license, see the header of `lib/icons.js`).
- **API**: `lib/api.js` speaks the REST API described in `docs/API.md`. Live updates read
  `GET /api/events` with `fetch` and a stream reader, because `EventSource` cannot send the
  `X-Api-Key` header.
- **Translations**: `_locales/<lang>/messages.json`. Messages use named placeholders
  (`$COUNT$`); `lib/i18n.js` fills them in **alphabetical order of their names**, so each
  placeholder's `content` must be `$1`, `$2`… in that order. Plurals are separate keys ending
  in `_one` / `_other`. Use typographic apostrophes (’) and, in French, « guillemets » with
  no-break spaces.
- **Debugging**: in the service worker console, `rustload.handleMenuClick({ menuItemId:
  'send-link', linkUrl: 'https://…' })` runs a menu action without right-clicking.
- After editing files, reload the extension from `chrome://extensions` (or *Reload* in
  `about:debugging`).

## Known limitations

- Firefox only grants the server permission from a click: if you revoke it in the browser's
  add-on settings, open the extension settings and click **Allow access** (or **Save**) again.
- Pages the browser protects (its own pages, extension stores, the built-in PDF viewer) can't
  be scanned for links; links inside cross-origin frames aren't collected either.
- The speed history in the popup starts when the popup opens (the server doesn't keep one).
- Clicks on `magnet:` links are handled in Firefox only; in Chromium browsers use the context
  menu or the paste box.
