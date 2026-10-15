// Dev-only fake backend: serves the API from memory and simulates live progress over a fake event
// stream, so the UI can be developed without the Rust server. Loaded only by `pnpm dev:mock`.

import { setTransport, type Package, type StatsEvent } from '$lib/api'
import { setEventStreamFactory, type EventStream } from '$lib/stores/live.svelte'
import * as seed from './data'
import * as extras from './extras'

type Handler = (match: RegExpMatchArray, body: any, query: URLSearchParams) => unknown

const MiB = 1024 ** 2
/** `localStorage['rustload.mock.empty'] = '1'` starts like a fresh install. */
const empty = localStorage.getItem('rustload.mock.empty') === '1'
const state = {
  authEnabled: true,
  setupRequired: false,
  user: null as { username: string } | null,
  paused: false,
  settings: structuredClone(seed.settings),
  categories: structuredClone(seed.categories),
  accounts: empty ? [] : structuredClone(seed.accounts),
  queue: empty ? [] : seed.seedQueue(),
  history: empty ? [] : seed.seedHistory(),
  logs: seed.seedLogs(),
  devicePolls: 0,
  ytdlpInstalled: false,
}

const listeners = new Set<MockStream>()
const emit = (type: string, data: unknown) => listeners.forEach((s) => s.dispatch(type, data))

class HttpError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message)
  }
}

const findPackage = (id: string) => {
  const pkg = state.queue.find((p) => p.id === Number(id)) ?? state.history.find((p) => p.id === Number(id))
  if (!pkg) throw new HttpError(404, 'NOT_FOUND', 'Package not found')
  return pkg
}
const findFile = (id: string) => {
  for (const p of state.queue) {
    const f = p.files.find((f) => f.id === Number(id))
    if (f) return { pkg: p, file: f }
  }
  throw new HttpError(404, 'NOT_FOUND', 'Download not found')
}
const system = () => ({
  version: '1.0.0-dev',
  download_root: '/downloads',
  uptime_secs: 93_784,
  extractor: '7zz',
  paused: state.paused,
  schedule_open: true,
  disk: { free: 1.42 * 1024 ** 4, total: 3.64 * 1024 ** 4 },
  disk_low: false,
})
const changed = () => emit('queue', {})
const resolveCategory = (c: (typeof state.categories)[number]) => ({
  ...c,
  resolved_path: c.path.startsWith('/') ? c.path : `/downloads${c.path ? `/${c.path}` : ''}`,
})

function addPackages(body: { links: string; name?: string; category_id?: number | null; paused?: boolean }): Package[] {
  const tokens = body.links.split(/\s+/).filter(Boolean)
  const links = tokens.filter((t) => /^https?:/.test(t))
  const magnets = tokens.filter((t) => t.startsWith('magnet:'))
  const created: Package[] = []
  if (links.length) {
    created.push(
      seed.makePackage(
        body.name || decodeURIComponent(links[0].split('/').pop() || 'Package'),
        (p) =>
          links.map((url) =>
            seed.makeFile(
              p,
              decodeURIComponent(url.split('/').pop() || 'file.bin'),
              Math.round((0.2 + Math.random() * 3) * 1024 ** 3),
              new URL(url).hostname,
              { url },
            ),
          ),
        { category_id: body.category_id ?? null, paused: !!body.paused, status: body.paused ? 'paused' : 'queued' },
      ),
    )
  }
  for (const magnet of magnets) {
    const dn = new URLSearchParams(magnet.slice(8)).get('dn') ?? 'Magnet'
    created.push(
      seed.makePackage(body.name || dn, () => [], {
        source_kind: 'magnet',
        status: 'debrid',
        debrid: { status: 'In queue', progress: 0, speed: 0, seeders: 0, size: null },
      }),
    )
  }
  state.queue.push(...created.map((p, i) => ({ ...p, position: state.queue.length + i })))
  changed()
  return created
}

const routes: [string, RegExp, Handler][] = [
  [
    'GET',
    /^auth\/status$/,
    () => ({ auth_enabled: state.authEnabled, setup_required: state.setupRequired, user: state.user }),
  ],
  [
    'POST',
    /^auth\/login$/,
    (_m, b) => {
      if (b.password !== 'password') throw new HttpError(401, 'INVALID_CREDENTIALS', 'Invalid credentials')
      state.user = { username: b.username }
      return { user: state.user }
    },
  ],
  [
    'POST',
    /^auth\/setup$/,
    (_m, b) => {
      state.setupRequired = false
      state.user = { username: b.username }
      return { user: state.user }
    },
  ],
  ['POST', /^auth\/logout$/, () => void (state.user = null)],
  [
    'PUT',
    /^auth\/password$/,
    (_m, b) => {
      if (b.current_password !== 'password') throw new HttpError(400, 'INVALID_CREDENTIALS', 'Wrong password')
    },
  ],
  ['GET', /^system$/, system],
  ['POST', /^queue\/pause$/, () => ((state.paused = true), system())],
  ['POST', /^queue\/resume$/, () => ((state.paused = false), system())],

  ['GET', /^packages$/, () => ({ packages: state.queue })],
  ['POST', /^packages$/, (_m, b) => ({ packages: addPackages(b) })],
  [
    'POST',
    /^packages\/batch$/,
    (_m, b) => ({ packages: (b.packages as Parameters<typeof addPackages>[0][]).flatMap(addPackages) }),
  ],
  [
    'POST',
    /^links\/check$/,
    (_m, b) => ({
      results: (b.urls as string[]).map((url) =>
        extras.checkLink(url, !!b.scan_pages, [...state.queue, ...state.history]),
      ),
    }),
  ],
  ['GET', /^media\/status$/, () => extras.mediaStatus(state.ytdlpInstalled)],
  ['POST', /^media\/install$/, () => ((state.ytdlpInstalled = true), extras.mediaStatus(true))],
  ['GET', /^stats$/, (_m, _b, q) => extras.stats(Number(q.get('days') ?? 30))],
  ['GET', /^system\/diagnostics$/, () => ({ checks: extras.diagnostics })],
  ['POST', /^backup\/restore$/, () => ({ settings: true, categories: 4, accounts: 2, packages: 3 })],
  [
    'POST',
    /^integrations\/telegram\/test$/,
    (_m, b) => {
      if (!String(b.token).includes(':')) throw new HttpError(400, 'TELEGRAM', 'Unauthorized: invalid token')
      return { username: 'rustload_home_bot' }
    },
  ],
  [
    'POST',
    /^packages\/reorder$/,
    (_m, b) => {
      state.queue.sort((a, c) => b.ids.indexOf(a.id) - b.ids.indexOf(c.id))
      changed()
    },
  ],
  [
    'PATCH',
    /^packages\/(\d+)$/,
    (m, b) => {
      const pkg = findPackage(m[1])
      if (pkg.dest_dir && ('category_id' in b || 'create_subfolder' in b))
        throw new HttpError(409, 'DEST_LOCKED', 'Destination is locked')
      Object.assign(pkg, b)
      changed()
      return pkg
    },
  ],
  [
    'DELETE',
    /^packages\/(\d+)$/,
    (m) => {
      state.queue = state.queue.filter((p) => p.id !== Number(m[1]))
      changed()
    },
  ],
  [
    'POST',
    /^packages\/(\d+)\/(pause|resume|retry|extract)$/,
    (m) => {
      const pkg = findPackage(m[1])
      if (m[2] === 'pause') Object.assign(pkg, { paused: true, status: 'paused' })
      if (m[2] === 'resume') Object.assign(pkg, { paused: false, status: 'queued' })
      if (m[2] === 'retry') {
        pkg.files.forEach(
          (f) => f.status === 'failed' && Object.assign(f, { status: 'queued', error: null, attempts: 0 }),
        )
        Object.assign(pkg, { status: 'queued', error: null })
      }
      for (const f of pkg.files)
        if (m[2] === 'pause' && f.status !== 'completed') Object.assign(f, { status: 'paused', speed: 0 })
      for (const f of pkg.files) if (m[2] === 'resume' && f.status === 'paused') f.status = 'queued'
      changed()
      return pkg
    },
  ],
  [
    'POST',
    /^packages\/(\d+)\/move$/,
    (m, b) => {
      const index = state.queue.findIndex((p) => p.id === Number(m[1]))
      const [pkg] = state.queue.splice(index, 1)
      const target = {
        top: 0,
        bottom: state.queue.length,
        up: Math.max(0, index - 1),
        down: Math.min(state.queue.length, index + 1),
      }[b.to as 'top']
      state.queue.splice(target, 0, pkg)
      changed()
    },
  ],
  [
    'POST',
    /^downloads\/(\d+)\/(pause|resume|retry)$/,
    (m) => {
      const { file } = findFile(m[1])
      file.status = m[2] === 'pause' ? 'paused' : 'queued'
      file.speed = 0
      file.error = null
      changed()
    },
  ],
  [
    'DELETE',
    /^downloads\/(\d+)$/,
    (m) => {
      const { pkg } = findFile(m[1])
      pkg.files = pkg.files.filter((f) => f.id !== Number(m[1]))
      changed()
    },
  ],

  [
    'GET',
    /^history$/,
    (_m, _b, q) => {
      const needle = (q.get('q') ?? '').toLowerCase()
      const items = state.history.filter((p) => p.name.toLowerCase().includes(needle))
      const offset = Number(q.get('offset') ?? 0)
      return { items: items.slice(offset, offset + Number(q.get('limit') ?? 50)), total: items.length }
    },
  ],
  ['DELETE', /^history$/, () => void ((state.history = []), emit('history', {}))],
  [
    'DELETE',
    /^history\/(\d+)$/,
    (m) => {
      state.history = state.history.filter((p) => p.id !== Number(m[1]))
      emit('history', {})
    },
  ],
  [
    'POST',
    /^history\/(\d+)\/redownload$/,
    (m) => {
      const pkg = findPackage(m[1])
      return { packages: addPackages({ links: pkg.files.map((f) => f.url).join('\n'), name: pkg.name }) }
    },
  ],

  ['GET', /^categories$/, () => ({ categories: state.categories.map(resolveCategory), download_root: '/downloads' })],
  [
    'POST',
    /^categories$/,
    (_m, b) => {
      const category = { ...b, id: seed.id(), position: state.categories.length, is_default: false, resolved_path: '' }
      state.categories.push(category)
      return resolveCategory(category)
    },
  ],
  [
    'POST',
    /^categories\/reorder$/,
    (_m, b) => {
      state.categories.sort((a, c) => b.ids.indexOf(a.id) - b.ids.indexOf(c.id))
    },
  ],
  [
    'PUT',
    /^categories\/(\d+)$/,
    (m, b) => {
      const category = state.categories.find((c) => c.id === Number(m[1]))!
      if (b.is_default) state.categories.forEach((c) => (c.is_default = false))
      Object.assign(category, b, { is_default: b.is_default || category.is_default })
      return resolveCategory(category)
    },
  ],
  [
    'DELETE',
    /^categories\/(\d+)$/,
    (m) => {
      if (state.categories.find((c) => c.id === Number(m[1]))?.is_default)
        throw new HttpError(409, 'DEFAULT_CATEGORY', 'The default category cannot be deleted')
      state.categories = state.categories.filter((c) => c.id !== Number(m[1]))
    },
  ],

  ['GET', /^accounts$/, () => ({ accounts: state.accounts, providers: seed.providers })],
  [
    'POST',
    /^accounts$/,
    (_m, b) => {
      if (String(b.api_key).length < 8) throw new HttpError(400, 'INVALID_API_KEY', 'Invalid API key')
      const account = {
        ...state.accounts[0],
        id: seed.id(),
        provider: b.provider,
        label: b.label ?? seed.providers.find((p) => p.id === b.provider)!.name,
        position: state.accounts.length,
        last_check: Math.floor(Date.now() / 1000),
      }
      state.accounts.push(account)
      return account
    },
  ],
  [
    'POST',
    /^accounts\/reorder$/,
    (_m, b) => {
      state.accounts.sort((a, c) => b.ids.indexOf(a.id) - b.ids.indexOf(c.id))
    },
  ],
  [
    'PUT',
    /^accounts\/(\d+)$/,
    (m, b) =>
      Object.assign(
        state.accounts.find((a) => a.id === Number(m[1]))!,
        b,
      ),
  ],
  ['DELETE', /^accounts\/(\d+)$/, (m) => void (state.accounts = state.accounts.filter((a) => a.id !== Number(m[1])))],
  [
    'POST',
    /^accounts\/(\d+)\/check$/,
    (m) =>
      Object.assign(
        state.accounts.find((a) => a.id === Number(m[1]))!,
        { last_check: Math.floor(Date.now() / 1000) },
      ),
  ],
  [
    'POST',
    /^accounts\/device\/(\w+)\/start$/,
    () => {
      state.devicePolls = 0
      return { user_code: 'K7QX-2M4P', verification_url: 'https://example.com/pin', expires_in: 600, poll_token: 'tok' }
    },
  ],
  [
    'POST',
    /^accounts\/device\/(\w+)\/poll$/,
    (m) => {
      if (++state.devicePolls < 3) return { status: 'pending', account: null }
      const account = {
        ...state.accounts[0],
        id: seed.id(),
        provider: m[1],
        label: m[1],
        position: state.accounts.length,
      }
      state.accounts.push(account)
      return { status: 'activated', account }
    },
  ],

  ['GET', /^logs$/, () => ({ entries: state.logs })],
  ['DELETE', /^logs$/, () => void (state.logs = [])],
  ['GET', /^settings$/, () => state.settings],
  [
    'PUT',
    /^settings$/,
    (_m, b) => {
      if (b.downloads.max_parallel > 20)
        throw new HttpError(400, 'INVALID_SETTINGS', 'max_parallel must be between 1 and 20')
      state.settings = b
      return state.settings
    },
  ],
  [
    'POST',
    /^settings\/api-key$/,
    () => {
      // 64 hex characters, like the server's `random_token()`.
      const bytes = crypto.getRandomValues(new Uint8Array(32))
      state.settings.integrations.api_key = Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
      return { api_key: state.settings.integrations.api_key }
    },
  ],
  [
    'POST',
    /^notifications\/test$/,
    (_m, b) => {
      if (!b.url && b.kind !== 'telegram') throw new HttpError(400, 'NOTIFY_FAILED', 'Connection refused')
    },
  ],
]

async function transport(url: string, init: RequestInit): Promise<Response> {
  await new Promise((r) => setTimeout(r, 80 + Math.random() * 120))
  const [path, qs] = url.replace(/^api\//, '').split('?')
  const method = init.method ?? 'GET'
  let body: unknown = undefined
  if (typeof init.body === 'string') body = JSON.parse(init.body)
  if (init.body instanceof FormData) {
    body = JSON.parse(String(init.body.get('options') ?? '{}'))
    const files = init.body.getAll('file') as File[]
    const created = files.map((f) =>
      seed.makePackage(f.name.replace(/\.torrent$/, ''), () => [], {
        source_kind: 'torrent',
        status: 'debrid',
        debrid: { status: 'In queue', progress: 0, speed: 0, seeders: 0, size: null },
      }),
    )
    state.queue.push(...created)
    changed()
    return json(200, { packages: created })
  }
  const authFree = path.startsWith('auth/')
  if (state.authEnabled && !state.user && !authFree) return json(401, { error: 'Unauthorized', code: 'UNAUTHORIZED' })
  for (const [m, re, handler] of routes) {
    const match = m === method ? path.match(re) : null
    if (!match) continue
    try {
      const result = handler(match, body, new URLSearchParams(qs))
      return result === undefined ? new Response(null, { status: 204 }) : json(200, result)
    } catch (err) {
      if (err instanceof HttpError) return json(err.status, { error: err.message, code: err.code })
      throw err
    }
  }
  return json(404, { error: `No mock route for ${method} ${path}`, code: 'NOT_FOUND' })
}

function json(status: number, data: unknown): Response {
  return new Response(JSON.stringify(data), { status, headers: { 'Content-Type': 'application/json' } })
}

class MockStream implements EventStream {
  onerror: ((event: Event) => void) | null = null
  onopen: ((event: Event) => void) | null = null
  #listeners = new Map<string, ((event: MessageEvent<string>) => void)[]>()

  constructor() {
    listeners.add(this)
    setTimeout(() => this.onopen?.(new Event('open')), 50)
  }

  addEventListener(type: string, listener: (event: MessageEvent<string>) => void): void {
    this.#listeners.set(type, [...(this.#listeners.get(type) ?? []), listener])
  }

  dispatch(type: string, data: unknown): void {
    const event = new MessageEvent<string>(type, { data: JSON.stringify(data) })
    this.#listeners.get(type)?.forEach((l) => l(event))
  }

  close(): void {
    listeners.delete(this)
  }
}

/** One simulation step per second: advance transfers, debrid jobs and emit stats. */
function tick() {
  const now = Math.floor(Date.now() / 1000)
  const max = state.settings.downloads.max_parallel
  const limit = state.settings.downloads.speed_limit_enabled
    ? state.settings.downloads.speed_limit_kib * 1024
    : Infinity
  const files = state.queue.filter((p) => !p.paused).flatMap((p) => p.files)
  let active = state.paused ? [] : files.filter((f) => f.status === 'downloading' || f.status === 'resolving')
  for (const f of files) {
    if (state.paused || active.length >= max) break
    if (f.status === 'queued') {
      f.status = 'resolving'
      f.started_at = now
      active.push(f)
    }
  }
  const share = Math.min(limit, 96 * MiB) / Math.max(1, active.length)
  for (const f of active) {
    if (f.status === 'resolving') {
      f.status = 'downloading'
      continue
    }
    f.connections = state.settings.downloads.connections
    f.speed = Math.round(share * (0.7 + Math.random() * 0.3))
    f.downloaded = Math.min(f.size ?? Infinity, f.downloaded + f.speed)
    f.eta = f.size ? (f.size - f.downloaded) / Math.max(1, f.speed) : null
    if (f.size && f.downloaded >= f.size)
      Object.assign(f, { status: 'completed', speed: 0, eta: null, finished_at: now, connections: 0 })
  }
  active = active.filter((f) => f.status === 'downloading' || f.status === 'resolving')

  for (const pkg of state.queue) {
    if (pkg.status === 'debrid' && pkg.debrid) {
      const progress = Math.min(1, pkg.debrid.progress + 0.01)
      pkg.debrid = {
        ...pkg.debrid,
        status: progress < 1 ? 'Downloading' : 'Ready',
        progress,
        speed: 40 * MiB,
        seeders: 64,
      }
    } else if (!pkg.paused && pkg.files.some((f) => f.status === 'downloading' || f.status === 'resolving')) {
      pkg.status = 'downloading'
    }
    if (pkg.files.length && pkg.files.every((f) => f.status === 'completed')) {
      pkg.status = 'completed'
      pkg.finished_at = now
    }
  }
  const finished = state.queue.filter((p) => p.status === 'completed')
  if (finished.length) {
    state.queue = state.queue.filter((p) => p.status !== 'completed')
    state.history.unshift(...finished)
    for (const p of finished)
      emit('notice', {
        level: 'success',
        key: 'package_finished',
        params: { name: p.name },
        message: `${p.name} finished`,
      })
    emit('queue', {})
    emit('history', {})
  }

  const remaining = state.queue
    .flatMap((p) => p.files)
    .reduce((s, f) => s + Math.max(0, (f.size ?? 0) - f.downloaded), 0)
  const speed = active.reduce((s, f) => s + f.speed, 0)
  const stats: StatsEvent = {
    speed,
    active: active.length,
    queued: state.queue.flatMap((p) => p.files).filter((f) => f.status === 'queued').length,
    paused: state.paused,
    schedule_open: true,
    speed_limit: Number.isFinite(limit) ? limit : 0,
    queue_remaining: remaining,
    eta: speed > 0 ? remaining / speed : null,
    disk_free: 1.42 * 1024 ** 4,
    disk_total: 3.64 * 1024 ** 4,
    disk_low: false,
    speed_rule_active: false,
  }
  emit('stats', stats)
  if (active.length || state.queue.some((p) => p.status === 'debrid')) {
    emit('progress', {
      downloads: active.map(({ id, package_id, status, downloaded, size, speed, eta, connections }) => ({
        id,
        package_id,
        status,
        downloaded,
        size,
        speed,
        eta,
        connections,
        ...(size && status === 'downloading'
          ? extras.segmentMap(downloaded / size, connections, id)
          : { map: null, heads: [] }),
      })),
      packages: state.queue.map(({ id, status, debrid, extract_progress, verify_progress }) => ({
        id,
        status,
        debrid,
        extract_progress,
        verify_progress,
      })),
    })
  }
  if (Math.random() < 0.3) {
    const entry = seed.logEntry()
    state.logs.push(entry)
    emit('log', entry)
  }
}

export function installMock(): void {
  setTransport(transport)
  setEventStreamFactory(() => new MockStream())
  setInterval(tick, 1000)
  console.info('[rustload] mock backend active — sign in with any username and the password "password"')
}
