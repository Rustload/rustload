/**
 * Connecting to the Rustload desktop app on this computer: find it on its local ports, then ask
 * it for its address and API key (the person allows it in the app). See `POST /api/desktop/pair`.
 */

/** Where the app's interface listens: 8800, or the next free port. */
const APP_ORIGIN = 'http://127.0.0.1'
const PORTS = [8800, 8801, 8802, 8803, 8804, 8805, 8806, 8807, 8808, 8809, 8810]
const PROBE_TIMEOUT_MS = 1500
/** The app waits up to 60 s for the person's answer. */
const PAIR_TIMEOUT_MS = 70000

/** Host-permission pattern the app needs (every port of 127.0.0.1). */
export const APP_PATTERN = `${APP_ORIGIN}/*`

/**
 * kind: `notRunning` | `serverHere` | `outdated` (an app without pairing) | `refused` | `timeout` |
 *       `busy` | `failed`
 */
export class PairingError extends Error {
  constructor(kind) {
    super(kind)
    this.name = 'PairingError'
    this.kind = kind
  }
}

/** The browser's name, shown in the app's question. */
export function browserName() {
  const brands = navigator.userAgentData?.brands?.map((b) => b.brand) ?? []
  for (const name of ['Microsoft Edge', 'Brave', 'Opera', 'Vivaldi', 'Google Chrome']) {
    if (brands.includes(name)) return name.replace(/^(Google|Microsoft) /, '')
  }
  if (navigator.userAgent.includes('Firefox/')) return 'Firefox'
  return brands.includes('Chromium') ? 'Chromium' : ''
}

/** `{ url, desktop }` when a Rustload answers on `port`. */
async function probe(port) {
  try {
    const response = await fetch(`${APP_ORIGIN}:${port}/api/auth/status`, {
      credentials: 'omit',
      cache: 'no-store',
      signal: AbortSignal.timeout(PROBE_TIMEOUT_MS),
    })
    const status = response.ok ? await response.json() : null
    if (typeof status?.auth_enabled !== 'boolean') return null
    return { url: `${APP_ORIGIN}:${port}`, desktop: status.desktop === true }
  } catch {
    return null
  }
}

/**
 * The desktop app's address. Throws `notRunning`, or `serverHere` (with `url`) when only a Rustload
 * server answers on this computer (in Docker, for instance): server mode is the one for it.
 */
export async function findApp() {
  const found = (await Promise.all(PORTS.map(probe))).filter(Boolean)
  const app = found.find((f) => f.desktop)
  if (app) return app.url
  const error = new PairingError(found.length ? 'serverHere' : 'notRunning')
  error.url = found[0]?.url ?? ''
  throw error
}

/**
 * Asks the app at `appUrl` to connect this extension. Resolves to `{ serverUrl, apiKey }` once the
 * person allowed it in the app.
 */
export async function pairWithApp(appUrl) {
  let response
  try {
    response = await fetch(`${appUrl}/api/desktop/pair`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
      body: JSON.stringify({ browser: browserName() }),
      credentials: 'omit',
      cache: 'no-store',
      signal: AbortSignal.timeout(PAIR_TIMEOUT_MS),
    })
  } catch (error) {
    throw new PairingError(error?.name === 'TimeoutError' ? 'timeout' : 'notRunning')
  }
  let payload = null
  try {
    payload = await response.json()
  } catch {
    payload = null
  }
  if (response.ok && typeof payload?.api_key === 'string' && typeof payload.url === 'string') {
    return { serverUrl: payload.url.replace(/\/+$/, ''), apiKey: payload.api_key }
  }
  const kind = {
    PAIRING_REFUSED: 'refused',
    PAIRING_TIMEOUT: 'timeout',
    PAIRING_IN_PROGRESS: 'busy',
    NOT_FOUND: 'outdated',
  }[payload?.code]
  throw new PairingError(kind ?? (response.status === 404 ? 'outdated' : 'failed'))
}
