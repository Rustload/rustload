/** Error raised for any failed API call. `code` comes from the server (`{ error, code }`) or is
 *  `NETWORK` when the server could not be reached. */
export class ApiError extends Error {
  readonly status: number
  readonly code: string

  constructor(status: number, code: string, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
  }
}

export type Method = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'

/** Swappable so the dev mock can serve requests without a backend. */
export type Transport = (url: string, init: RequestInit) => Promise<Response>

let transport: Transport = (url, init) => fetch(url, init)
let unauthorizedHandler: (() => void) | null = null

export function setTransport(next: Transport): void {
  transport = next
}

export function onUnauthorized(handler: () => void): void {
  unauthorizedHandler = handler
}

/** Builds a relative URL (no leading slash) so the app works under a reverse-proxy sub-path. */
export function apiUrl(path: string, query?: Record<string, string | number | boolean | undefined>): string {
  let url = `api/${path}`
  if (query) {
    const params = new URLSearchParams()
    for (const [key, value] of Object.entries(query)) {
      if (value !== undefined && value !== '') params.set(key, String(value))
    }
    const qs = params.toString()
    if (qs) url += `?${qs}`
  }
  return url
}

export async function request<T>(method: Method, url: string, body?: unknown): Promise<T> {
  // The custom header proves the request comes from this page: other sites cannot set it
  // without a CORS preflight, which the server never grants (CSRF protection).
  const headers: Record<string, string> = { Accept: 'application/json', 'X-Rustload-Request': '1' }
  const init: RequestInit = { method, credentials: 'same-origin', headers }

  if (body instanceof FormData) {
    init.body = body
  } else if (body !== undefined) {
    headers['Content-Type'] = 'application/json'
    init.body = JSON.stringify(body)
  }

  let res: Response
  try {
    res = await transport(url, init)
  } catch (err) {
    throw new ApiError(0, 'NETWORK', err instanceof Error ? err.message : String(err))
  }

  if (res.status === 401 && !url.startsWith('api/auth/')) unauthorizedHandler?.()

  const text = res.status === 204 ? '' : await res.text()
  let data: unknown = undefined
  if (text) {
    try {
      data = JSON.parse(text)
    } catch {
      data = undefined
    }
  }

  if (!res.ok) {
    const payload = (data ?? {}) as { error?: string; code?: string }
    throw new ApiError(res.status, payload.code ?? `HTTP_${res.status}`, payload.error ?? res.statusText)
  }

  return data as T
}
