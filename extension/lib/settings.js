import { ext } from './browser.js'

/** Everything the extension stores, in `storage.local` only (never synced, never logged). */
export const DEFAULTS = Object.freeze({
  serverUrl: '',
  apiKey: '',
  addPaused: false,
  /** null = automatic category (chosen by Rustload from the file extensions). */
  categoryId: null,
  /** Last known name of the category, to show it while the server is offline. */
  categoryName: '',
  notify: true,
})

export async function loadSettings() {
  return { ...DEFAULTS, ...(await ext.storage.local.get({ ...DEFAULTS })) }
}

export function saveSettings(values) {
  return ext.storage.local.set(values)
}

/**
 * Validates and normalises a server address typed by the user.
 * Returns `{ url }` (origin + optional reverse-proxy path, no trailing slash) or `{ error }` with
 * `empty`, `invalid`, `scheme` or `credentials`.
 */
export function parseServerUrl(input) {
  let text = String(input ?? '').trim()
  if (!text) return { error: 'empty' }
  // "nas:8800" or "192.168.1.5:8800": assume http, the default for a server on the LAN.
  if (!/^[a-z][a-z\d+.-]*:\/\//i.test(text)) text = `http://${text}`
  let url
  try {
    url = new URL(text)
  } catch {
    return { error: 'invalid' }
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return { error: 'scheme' }
  if (url.username || url.password) return { error: 'credentials' }
  if (!url.hostname || /\s/.test(url.hostname)) return { error: 'invalid' }
  // Keep a reverse-proxy sub-path, drop query/hash and a pasted "/api" suffix.
  const path = url.pathname.replace(/\/+$/, '').replace(/\/api$/i, '')
  return { url: `${url.origin}${path}` }
}

/**
 * Host-permission match pattern for the server. The port is left out on purpose: Chrome then
 * matches every port of that host and Firefox ignores ports in match patterns anyway.
 */
export function originPattern(serverUrl) {
  const url = new URL(serverUrl)
  return `${url.protocol}//${url.hostname}/*`
}

export async function hasHostPermission(serverUrl) {
  try {
    return await ext.permissions.contains({ origins: [originPattern(serverUrl)] })
  } catch {
    return false
  }
}

/** "nas:8800" — what the user recognises in compact places. */
export function displayHost(serverUrl) {
  try {
    return new URL(serverUrl).host
  } catch {
    return serverUrl
  }
}
