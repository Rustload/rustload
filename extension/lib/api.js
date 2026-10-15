/**
 * Minimal client for the Rustload REST API (docs/API.md), authenticated with `X-Api-Key`.
 * Every failure becomes an `ApiError` whose `kind` tells the UI exactly what went wrong.
 */

const TIMEOUT_MS = 8000
/**
 * Adding expands playlists and folders (up to 60 s each) and waits for the debrid service with
 * magnets: give it much more time.
 */
const ADD_TIMEOUT_MS = 180000

/**
 * kind: `unconfigured` | `permission` | `unreachable` | `timeout` | `auth` | `refused` (401 that
 *       isn't Rustload's, e.g. a reverse proxy's login) | `notRustload` | `server` (5xx) |
 *       `rejected` (4xx with a Rustload error code)
 * `maybeAdded`: an add timed out, the server may still have added the links.
 */
export class ApiError extends Error {
  constructor(kind, { status = 0, code = '', detail = '', seconds = 0, missingKey = false, maybeAdded = false } = {}) {
    super(detail || code || kind)
    this.name = 'ApiError'
    this.kind = kind
    this.status = status
    this.code = code
    this.detail = detail
    this.seconds = seconds
    this.missingKey = missingKey
    this.maybeAdded = maybeAdded
  }
}

export function createClient({ serverUrl, apiKey }) {
  function headers(extra = {}) {
    const result = { Accept: 'application/json', ...extra }
    if (apiKey) result['X-Api-Key'] = apiKey
    return result
  }

  async function errorFrom(response) {
    let payload = null
    if ((response.headers.get('content-type') ?? '').includes('application/json')) {
      try {
        payload = await response.json()
      } catch {
        payload = null
      }
    }
    const status = response.status
    const code = typeof payload?.code === 'string' ? payload.code : ''
    const detail = typeof payload?.error === 'string' ? payload.error : ''
    // Only Rustload's own 401 means a wrong key: a reverse proxy in front may ask for its login.
    if (status === 401 && code === 'UNAUTHORIZED') return new ApiError('auth', { status, code, missingKey: !apiKey })
    if (status === 401) return new ApiError('refused', { status })
    // Without Rustload's `{ error, code }` body, something else answered: a proxy, another app,
    // or a wrong path.
    if (!code) return new ApiError(status >= 500 ? 'server' : 'notRustload', { status })
    return new ApiError(status >= 500 ? 'server' : 'rejected', { status, code, detail })
  }

  async function request(path, { method = 'GET', body, timeout = TIMEOUT_MS } = {}) {
    const extra = {}
    if (body !== undefined) extra['Content-Type'] = 'application/json'
    // Only checked when the server runs without authentication (behind an authenticating proxy):
    // the extension is a legitimate client, not a cross-site page.
    if (method !== 'GET') extra['X-Rustload-Request'] = '1'
    const timer = AbortSignal.timeout(timeout)
    let response
    try {
      response = await fetch(serverUrl + path, {
        method,
        headers: headers(extra),
        body: body === undefined ? undefined : JSON.stringify(body),
        credentials: 'omit',
        cache: 'no-store',
        signal: timer,
      })
    } catch {
      if (timer.aborted) throw new ApiError('timeout', { seconds: Math.round(timeout / 1000) })
      throw new ApiError('unreachable')
    }
    if (!response.ok) throw await errorFrom(response)
    if (response.status === 204) return null
    if (!(response.headers.get('content-type') ?? '').includes('application/json')) {
      throw new ApiError('notRustload', { status: response.status })
    }
    try {
      return await response.json()
    } catch {
      throw new ApiError('notRustload', { status: response.status })
    }
  }

  async function system() {
    const data = await request('/api/system')
    if (typeof data?.version !== 'string' || typeof data.paused !== 'boolean') throw new ApiError('notRustload')
    return data
  }

  /**
   * Reads `GET /api/events` (Server-Sent Events) with fetch, because EventSource cannot send the
   * `X-Api-Key` header. Resolves never: it rejects when the stream fails or ends.
   */
  async function events({ signal, onEvent }) {
    let response
    try {
      response = await fetch(`${serverUrl}/api/events`, {
        headers: headers({ Accept: 'text/event-stream' }),
        credentials: 'omit',
        cache: 'no-store',
        signal,
      })
    } catch (error) {
      if (signal?.aborted) throw error
      throw new ApiError('unreachable')
    }
    if (!response.ok) throw await errorFrom(response)
    if (!(response.headers.get('content-type') ?? '').startsWith('text/event-stream') || !response.body) {
      throw new ApiError('notRustload', { status: response.status })
    }

    const reader = response.body.pipeThrough(new TextDecoderStream()).getReader()
    let buffer = ''
    let name = ''
    let data = []
    try {
      for (;;) {
        const { value, done } = await reader.read()
        if (done) break
        buffer += value
        const lines = buffer.split('\n')
        buffer = lines.pop() ?? ''
        for (let line of lines) {
          if (line.endsWith('\r')) line = line.slice(0, -1)
          if (line === '') {
            if (data.length) {
              let payload = {}
              try {
                payload = JSON.parse(data.join('\n'))
              } catch {
                payload = {}
              }
              try {
                onEvent(name || 'message', payload)
              } catch (error) {
                // A failing handler must not end the stream: report it and keep reading.
                reportError(error)
              }
            }
            name = ''
            data = []
          } else if (!line.startsWith(':')) {
            const colon = line.indexOf(':')
            const field = colon === -1 ? line : line.slice(0, colon)
            let fieldValue = colon === -1 ? '' : line.slice(colon + 1)
            if (fieldValue.startsWith(' ')) fieldValue = fieldValue.slice(1)
            if (field === 'event') name = fieldValue
            else if (field === 'data') data.push(fieldValue)
          }
        }
      }
    } finally {
      // Whatever ended the loop, release the connection.
      reader.cancel().catch(() => {})
    }
    throw new ApiError('unreachable')
  }

  return {
    system,
    events,
    packages: async () => (await request('/api/packages'))?.packages ?? [],
    categories: async () => (await request('/api/categories'))?.categories ?? [],
    /** `links` is a list of URLs/magnets; returns the created packages. */
    add: async (links, { paused = false, categoryId = null } = {}) => {
      const body = { links: links.join('\n'), paused: Boolean(paused) }
      if (categoryId != null) body.category_id = categoryId
      try {
        return (await request('/api/packages', { method: 'POST', body, timeout: ADD_TIMEOUT_MS }))?.packages ?? []
      } catch (error) {
        // No answer in time doesn't mean nothing was added: the server may still be at work.
        if (error.kind === 'timeout') throw new ApiError('timeout', { seconds: error.seconds, maybeAdded: true })
        throw error
      }
    },
    pause: () => request('/api/queue/pause', { method: 'POST' }),
    resume: () => request('/api/queue/resume', { method: 'POST' }),
  }
}
