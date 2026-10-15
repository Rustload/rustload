# Rustload web UI

Svelte 5 + TypeScript + Tailwind CSS 4, built with Vite. The production build (`dist/`) is embedded
in the Rustload binary and served from `/`.

```sh
pnpm install
pnpm dev        # dev server on :5173, proxies /api to http://127.0.0.1:8800 (override with RUSTLOAD_BACKEND)
pnpm dev:mock   # same, with an in-browser fake backend (sign in with any user / "password")
pnpm build      # outputs dist/
pnpm lint       # prettier --check + svelte-check
pnpm format     # prettier --write
```

## Layout

```
src/
  App.svelte, Shell.svelte     auth gate, app shell, global shortcuts (N, paste, drop)
  pages/                       one component per route (+ pages/settings/* sections)
  lib/api/                     typed client: request() helper, ApiError, endpoint map, types
  lib/stores/                  runes-based stores (queue, history, logs, settings, live SSE…)
  lib/i18n/                    en.ts (source of truth), fr.ts, t() with plurals
  lib/components/              ui/ primitives + feature components
  lib/utils/                   formatting, link parsing, status colours, clipboard
  mock/                        dev-only fake API and event stream (never in production builds)
```

The API contract lives in [`../docs/API.md`](../docs/API.md). Design notes: [`DESIGN.md`](DESIGN.md).

## Translations

Add a language by copying `src/lib/i18n/fr.ts`, typing it as `Messages`, and registering it in
`src/lib/i18n/index.svelte.ts`. TypeScript reports any missing key.
