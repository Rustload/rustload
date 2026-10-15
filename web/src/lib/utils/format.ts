import { i18n, t } from '$lib/i18n/index.svelte'

/** No-break space between a number and its unit. */
const UNIT_SPACE = String.fromCharCode(0xa0)

const UNIT_KEYS = ['units.b', 'units.kib', 'units.mib', 'units.gib', 'units.tib'] as const

const numberFormats = new Map<string, Intl.NumberFormat>()
function numberFormat(digits: number, trim: boolean, style: 'decimal' | 'percent' = 'decimal'): Intl.NumberFormat {
  const key = `${i18n.tag}:${style}:${digits}:${trim}`
  let fmt = numberFormats.get(key)
  if (!fmt) {
    fmt = new Intl.NumberFormat(i18n.tag, {
      style,
      minimumFractionDigits: trim ? 0 : digits,
      maximumFractionDigits: digits,
    })
    numberFormats.set(key, fmt)
  }
  return fmt
}

export function formatNumber(value: number, digits = 0): string {
  return numberFormat(digits, false).format(value)
}

type SizeOptions = {
  /** Drop trailing zeros ("5 MiB" instead of "5.00 MiB"), for axis labels and settings hints. */
  trim?: boolean
}

/** Splits a byte count into a display value and a binary unit (KiB, MiB…). Fixed decimals keep live
 *  values from jittering. */
export function splitBytes(bytes: number, { trim = false }: SizeOptions = {}): { value: string; unit: string } {
  let size = Math.max(0, bytes)
  let unit = 0
  while (size >= 1024 && unit < UNIT_KEYS.length - 1) {
    size /= 1024
    unit++
  }
  const digits = unit === 0 ? 0 : size < 10 ? 2 : size < 100 ? 1 : 0
  return { value: numberFormat(digits, trim).format(size), unit: t(UNIT_KEYS[unit]) }
}

export function formatBytes(bytes: number | null | undefined, options?: SizeOptions): string {
  if (bytes == null) return '—'
  const { value, unit } = splitBytes(bytes, options)
  return `${value}${UNIT_SPACE}${unit}`
}

/** Speed limits are set in KiB/s. */
export function kibToBytes(kib: number): number {
  return kib * 1024
}

export function formatSpeed(bytesPerSecond: number | null | undefined, options?: SizeOptions): string {
  return t('units.perSecond', { value: formatBytes(bytesPerSecond ?? 0, options) })
}

export function splitSpeed(bytesPerSecond: number): { value: string; unit: string } {
  const { value, unit } = splitBytes(bytesPerSecond)
  return { value, unit: t('units.perSecond', { value: unit }) }
}

/** Rounds up to 1, 2 or 5 × 10ⁿ within the binary unit the value is displayed in (e.g. 4.7 MiB → 5 MiB). */
export function niceBytesCeil(bytes: number): number {
  if (bytes <= 0) return 1024 * 1024
  const base = 1024 ** Math.min(UNIT_KEYS.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)))
  const mantissa = bytes / base
  const exp = 10 ** Math.floor(Math.log10(mantissa))
  const m = mantissa / exp
  return (m <= 1 ? 1 : m <= 2 ? 2 : m <= 5 ? 5 : 10) * exp * base
}

export function formatPercent(ratio: number | null | undefined): string {
  if (ratio == null || !Number.isFinite(ratio)) return '—'
  return numberFormat(ratio < 0.1 ? 1 : 0, true, 'percent').format(Math.min(1, Math.max(0, ratio)))
}

/** Human duration with the two most significant units, e.g. "1 h 12 min". */
export function formatDuration(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds)) return '—'
  const s = Math.max(0, Math.round(seconds))
  const parts: [number, 'units.day' | 'units.hour' | 'units.minute' | 'units.second'][] = [
    [Math.floor(s / 86400), 'units.day'],
    [Math.floor((s % 86400) / 3600), 'units.hour'],
    [Math.floor((s % 3600) / 60), 'units.minute'],
    [s % 60, 'units.second'],
  ]
  const first = parts.findIndex(([n]) => n > 0)
  if (first === -1) return t('units.second', { n: 0 })
  return parts
    .slice(first, first + 2)
    .filter(([n], i) => i === 0 || n > 0)
    .map(([n, key]) => t(key, { n }))
    .join(' ')
}

const clockFormats = new Map<string, Intl.DateTimeFormat>()
function dateFormat(options: Intl.DateTimeFormatOptions, id: string): Intl.DateTimeFormat {
  const key = `${i18n.tag}:${id}`
  let fmt = clockFormats.get(key)
  if (!fmt) {
    fmt = new Intl.DateTimeFormat(i18n.tag, options)
    clockFormats.set(key, fmt)
  }
  return fmt
}

const CALENDAR = {
  /** 14 Oct */
  day: { day: 'numeric', month: 'short' },
  /** Tuesday 14 October */
  longDay: { weekday: 'long', day: 'numeric', month: 'long' },
  /** Tue 14 Oct */
  shortDay: { weekday: 'short', day: 'numeric', month: 'short' },
  /** Tuesday */
  weekday: { weekday: 'long' },
  /** Oct */
  month: { month: 'short' },
  /** October 2026 */
  longMonth: { month: 'long', year: 'numeric' },
} satisfies Record<string, Intl.DateTimeFormatOptions>

export type CalendarStyle = keyof typeof CALENDAR

export function formatCalendar(date: Date, style: CalendarStyle): string {
  return dateFormat(CALENDAR[style], `calendar:${style}`).format(date)
}

export function formatClock(date: Date): string {
  return dateFormat({ hour: '2-digit', minute: '2-digit' }, 'clock').format(date)
}

export function formatDate(unixSeconds: number | null | undefined): string {
  if (unixSeconds == null) return '—'
  return dateFormat({ day: 'numeric', month: 'short', year: 'numeric' }, 'date').format(new Date(unixSeconds * 1000))
}

export function formatDateTime(unixSeconds: number | null | undefined): string {
  if (unixSeconds == null) return '—'
  return dateFormat({ dateStyle: 'medium', timeStyle: 'short' }, 'datetime').format(new Date(unixSeconds * 1000))
}

export function formatLogTime(ms: number): string {
  return dateFormat({ hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false }, 'log').format(
    new Date(ms),
  )
}

/** When something `etaSeconds` away ends: its time, and its day (in `dayStyle`) unless that's today. */
export function finishAt(
  etaSeconds: number | null | undefined,
  now: number,
  dayStyle: CalendarStyle = 'shortDay',
): { time: string; day: string | null } | null {
  if (etaSeconds == null || !Number.isFinite(etaSeconds)) return null
  const end = new Date(now + etaSeconds * 1000)
  const sameDay = end.toDateString() === new Date(now).toDateString()
  return { time: formatClock(end), day: sameDay ? null : formatCalendar(end, dayStyle) }
}

/** "done at 14:32", or with the date when it ends on another day. */
export function formatFinish(etaSeconds: number | null | undefined, now = Date.now()): string | null {
  const finish = finishAt(etaSeconds, now)
  if (!finish) return null
  return finish.day === null
    ? t('time.doneAt', { time: finish.time })
    : t('time.doneOn', { date: finish.day, time: finish.time })
}

export function formatRelative(unixSeconds: number | null | undefined, now = Date.now()): string {
  if (unixSeconds == null) return t('common.never')
  const diff = unixSeconds - now / 1000
  const abs = Math.abs(diff)
  if (abs < 45) return t('time.justNow')
  const [, unit, divisor] = RELATIVE_STEPS.find(([limit]) => abs < limit)!
  return relativeFormat().format(Math.round(diff / divisor), unit)
}

const relativeFormats = new Map<string, Intl.RelativeTimeFormat>()
function relativeFormat(): Intl.RelativeTimeFormat {
  let fmt = relativeFormats.get(i18n.tag)
  if (!fmt) {
    fmt = new Intl.RelativeTimeFormat(i18n.tag, { numeric: 'auto' })
    relativeFormats.set(i18n.tag, fmt)
  }
  return fmt
}

/** [upper bound in seconds, unit, seconds per unit] */
const RELATIVE_STEPS: [number, Intl.RelativeTimeFormatUnit, number][] = [
  [60, 'second', 1],
  [3600, 'minute', 60],
  [86400, 'hour', 3600],
  [604800, 'day', 86400],
  [2629800, 'week', 604800],
  [31557600, 'month', 2629800],
  [Infinity, 'year', 31557600],
]

export function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return url
  }
}

export function displayName(file: {
  filename: string | null
  url: string
  media?: { title: string | null } | null
}): string {
  if (file.filename) return file.filename
  if (file.media?.title) return file.media.title
  try {
    const u = new URL(file.url)
    const last = u.pathname.split('/').filter(Boolean).pop()
    return last ? decodeURIComponent(last) : u.hostname
  } catch {
    return file.url
  }
}
