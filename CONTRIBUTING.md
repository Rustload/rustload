# Contributing

Thanks for your interest in Rustload! Bug reports, ideas and pull requests are welcome.

## Development setup

You need Rust (stable, 1.94 or newer), Node.js 22.12+ with pnpm, and optionally 7-Zip in `PATH` to
test RAR/7z extraction.

```sh
# Backend, on http://127.0.0.1:8800 (data in ./config and ./downloads)
cargo run

# Frontend with hot reload, on http://127.0.0.1:5173, proxying /api to the backend
cd web
pnpm install
pnpm dev
```

`pnpm dev:mock` runs the interface against an in-browser fake backend (password `password`), handy
for UI work without downloading anything. In debug builds the backend serves `web/dist` from disk:
run `pnpm build` to see interface changes through port 8800.

## Checks

Run before opening a pull request — CI runs the same commands:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

cd web
pnpm lint      # Prettier + svelte-check
pnpm build
```

`cargo test` includes end-to-end tests of the transfer engine against a local HTTP server
(multi-connection, servers without range support, connection limits, resume).

When `Cargo.lock` changes (and before each release), regenerate the license texts of the
dependencies with [cargo-about](https://github.com/EmbarkStudios/cargo-about) (`cargo install
cargo-about --locked --features cli`):

```sh
cargo about generate --fail --locked about.hbs -o THIRD-PARTY-LICENSES.md
```

New dependencies must use one of the licenses accepted in `about.toml`; the web interface's
bundled packages and the typeface are written in `about.hbs`. CI does not check the file, so that
Dependabot updates are not blocked on it.

## Guidelines

- Keep the binary small and the hot path allocation-free; prefer the standard library and existing
  dependencies over new ones.
- Errors shown to users must be actionable ("not enough disk space: 4.2 GiB needed, 1.1 GiB free"),
  logs must never contain secrets (API keys, tokens, passwords).
- New settings need a default, validation in `Settings::validate`, an entry in `docs/API.md` and
  `docs/configuration.md`, and UI strings in both `web/src/lib/i18n/en.ts` and `fr.ts`.
- UI: reuse the primitives in `web/src/lib/components/ui/` and follow `web/DESIGN.md`.
- Comments explain *why*, not *what*.

## Browser extension

`extension/` is a Manifest V3 extension with no build step: load the folder unpacked in Chrome
(*chrome://extensions*, developer mode) or as a temporary add-on in Firefox (*about:debugging*).
It talks to the API with the API key and follows the same design rules as the web interface. CI
runs `npx web-ext lint --source-dir extension`. See [extension/README.md](extension/README.md).

## Desktop app

`desktop/` is a [Tauri](https://tauri.app) app in a Cargo workspace of its own: run its checks
there (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`). It embeds
the interface from `web/dist`, so build that first. On Linux it needs the WebKitGTK headers
(`libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev`).

```sh
cd web && pnpm build && cd ../desktop
cargo run                                         # the app, with a console for its logs
npx @tauri-apps/cli@2.12.3 build --bundles nsis --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

The release workflow signs the update files with the updater key (`TAURI_SIGNING_PRIVATE_KEY`
secret); local installers skip them as above. To try an update, point a debug build at a local
`latest.json` with `RUSTLOAD_UPDATE_URL=http://127.0.0.1:…/latest.json` (built by
`.github/scripts/update_manifest.py` from signed files).

Its data lives in the system's app data folder (`%LOCALAPPDATA%\io.github.rustload`,
`~/Library/Application Support/io.github.rustload`, `~/.local/share/io.github.rustload`): delete
`desktop.json` there to see the first-start page again. Its dependencies' licenses go to
`desktop/THIRD-PARTY-LICENSES.md`, with the same cargo-about command run in `desktop/`:
`cargo about generate --fail --locked -c ../about.toml ../about.hbs -o THIRD-PARTY-LICENSES.md`.

## Adding providers and plugins

See [docs/architecture.md](docs/architecture.md#adding-a-debrid-provider) for debrid providers and
hoster plugins. Please include unit tests for response parsing and error mapping (see the existing
`tests` modules).

## Reporting bugs

Open an issue with the Rustload version (*Settings › About*), how you run it (Docker, binary),
what you expected and what happened. Logs help (*Logs › Download log file*) — check them for
anything private first.

Questions and ideas are welcome in [Discussions](https://github.com/rustload/rustload/discussions).
Security issues: please follow [SECURITY.md](SECURITY.md) instead of opening a public issue.
