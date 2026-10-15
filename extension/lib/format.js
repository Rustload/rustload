import { locale, t } from './i18n.js'

/** No-break space between a number and its unit. */
const UNIT_SPACE = ' '
const UNIT_KEYS = ['unitB', 'unitKiB', 'unitMiB', 'unitGiB', 'unitTiB']

const formats = new Map()
function numberFormat(digits, trim = false) {
  const key = `${digits}:${trim}`
  let format = formats.get(key)
  if (!format) {
    format = new Intl.NumberFormat(locale, {
      minimumFractionDigits: trim ? 0 : digits,
      maximumFractionDigits: digits,
    })
    formats.set(key, format)
  }
  return format
}

/** Splits a byte count into a value and a binary unit. Fixed decimals keep live values steady. */
export function splitBytes(bytes, { trim = false } = {}) {
  let size = Math.max(0, bytes || 0)
  let unit = 0
  while (size >= 1024 && unit < UNIT_KEYS.length - 1) {
    size /= 1024
    unit++
  }
  const digits = unit === 0 ? 0 : size < 10 ? 2 : size < 100 ? 1 : 0
  return { value: numberFormat(digits, trim).format(size), unit: t(UNIT_KEYS[unit]) }
}

export function formatBytes(bytes, options) {
  if (bytes == null) return '—'
  const { value, unit } = splitBytes(bytes, options)
  return `${value}${UNIT_SPACE}${unit}`
}

export function splitSpeed(bytesPerSecond) {
  const { value, unit } = splitBytes(bytesPerSecond)
  return { value, unit: t('perSecond', { value: unit }) }
}

export function formatSpeed(bytesPerSecond, options) {
  return t('perSecond', { value: formatBytes(bytesPerSecond ?? 0, options) })
}

/** Rounds up to 1, 2 or 5 × 10ⁿ within the displayed binary unit (4.7 MiB → 5 MiB). */
export function niceBytesCeil(bytes) {
  if (bytes <= 0) return 1024 * 1024
  const base = 1024 ** Math.min(UNIT_KEYS.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)))
  const mantissa = bytes / base
  const exp = 10 ** Math.floor(Math.log10(mantissa))
  const m = mantissa / exp
  return (m <= 1 ? 1 : m <= 2 ? 2 : m <= 5 ? 5 : 10) * exp * base
}

export function formatPercent(ratio) {
  if (ratio == null || !Number.isFinite(ratio)) return '—'
  return new Intl.NumberFormat(locale, { style: 'percent', maximumFractionDigits: ratio < 0.1 ? 1 : 0 }).format(
    Math.min(1, Math.max(0, ratio)),
  )
}

/** Two most significant units, e.g. "1 h 12 min". */
export function formatDuration(seconds) {
  if (seconds == null || !Number.isFinite(seconds)) return '—'
  const s = Math.max(0, Math.round(seconds))
  const parts = [
    [Math.floor(s / 86400), 'durDay'],
    [Math.floor((s % 86400) / 3600), 'durHour'],
    [Math.floor((s % 3600) / 60), 'durMinute'],
    [s % 60, 'durSecond'],
  ]
  const first = parts.findIndex(([n]) => n > 0)
  if (first === -1) return t('durSecond', { n: 0 })
  return parts
    .slice(first, first + 2)
    .filter(([n], i) => i === 0 || n > 0)
    .map(([n, key]) => t(key, { n }))
    .join(' ')
}

/**
 * File name of a queue entry, its media title (yt-dlp), or the last part of its URL until the
 * server knows them.
 */
export function displayName(file) {
  if (file.filename) return file.filename
  if (file.media?.title) return file.media.title
  try {
    const url = new URL(file.url)
    const last = url.pathname.split('/').filter(Boolean).pop()
    return last ? decodeURIComponent(last) : url.hostname
  } catch {
    return file.url
  }
}
