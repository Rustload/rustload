/** Reading the server's link checks (`POST /api/links/check`), shared by the popup and the picker. */

import { CHECK_BATCH } from './api.js'
import { formatBytes } from './format.js'
import { t } from './i18n.js'

/** Handlers that download the link itself, rather than leaving a web page. */
const DOWNLOADERS = new Set(['debrid', 'hoster', 'media', 'playlist', 'magnet'])

/** Checks `urls` in batches, giving each batch's results to `onResults` as they arrive. */
export async function checkInBatches(client, urls, onResults) {
  for (let start = 0; start < urls.length; start += CHECK_BATCH) {
    onResults(await client.check(urls.slice(start, start + CHECK_BATCH)))
  }
}

/** Whether an account, a plugin or yt-dlp takes the link (a download page of a known host). */
export function hasDownloader(result) {
  return DOWNLOADERS.has(result?.handler?.type)
}

/** `ok` online, `bad` offline or not downloadable, `muted` unknown, `checking` without result. */
export function checkTone(result) {
  if (!result) return 'checking'
  if (result.status === 'online') return 'ok'
  if (result.status === 'offline' || result.status === 'unsupported') return 'bad'
  return 'muted'
}

/** Who downloads the link: "via AllDebrid", "YouTube", "direct download". */
export function checkVia(result) {
  const handler = result?.handler
  if (!handler) return ''
  if (handler.type === 'debrid' || handler.type === 'hoster' || handler.type === 'magnet') {
    return t('checkVia', { name: handler.name })
  }
  if (handler.type === 'media' || handler.type === 'playlist') return handler.name
  if (handler.type === 'direct') return t('checkDirect')
  return ''
}

/** "Online · 1.6 GiB · via AllDebrid", "Offline: file deleted", "already downloaded"… */
export function checkSummary(result, { size = true } = {}) {
  if (!result) return t('checkChecking')
  const parts = []
  if (result.status === 'online') parts.push(t('checkOnline'))
  else if (result.status === 'offline') parts.push(result.error ? t('checkOfflineWhy', { reason: result.error }) : t('checkOffline'))
  else if (result.status === 'unsupported') parts.push(t('checkUnsupported'))
  if (size && result.size) parts.push(formatBytes(result.size))
  // Who would download a dead link or a web page does not matter.
  if (result.status === 'online' || result.status === 'unknown') {
    const via = checkVia(result)
    if (via) parts.push(via)
  }
  if (result.duplicate) {
    parts.push(t(result.duplicate.location === 'history' ? 'checkInHistory' : 'checkInQueue'))
  }
  return parts.join(' · ')
}
