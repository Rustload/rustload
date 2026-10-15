import { api, type LogEntry, type NoticeEvent, type ProgressEvent, type StatsEvent } from '$lib/api'
import { hasMessage, t } from '$lib/i18n/index.svelte'
import { formatBytes } from '$lib/utils/format'
import { auth } from './auth.svelte'
import { desktop } from './desktop.svelte'
import { history } from './history.svelte'
import { logs } from './logs.svelte'
import { queue } from './queue.svelte'
import { system } from './system.svelte'
import { toasts } from './toasts.svelte'

/** The subset of EventSource used here, so the dev mock can provide its own. */
export interface EventStream {
  addEventListener(type: string, listener: (event: MessageEvent<string>) => void): void
  onerror: ((event: Event) => void) | null
  onopen: ((event: Event) => void) | null
  close(): void
}

let createStream = (url: string): EventStream => new EventSource(url, { withCredentials: true }) as EventStream

export function setEventStreamFactory(factory: (url: string) => EventStream): void {
  createStream = factory
}

const BACKOFF_MS = [1000, 2000, 4000, 8000, 15000]

function noticeText(event: NoticeEvent): string {
  const key = `notices.${event.key}`
  if (!hasMessage(key)) return event.message
  // Sizes come as raw byte counts.
  const params = { ...event.params }
  if (params.free && /^\d+$/.test(params.free)) params.free = formatBytes(Number(params.free))
  return t(key, params)
}

/** Single shared connection to `GET /api/events`, dispatching each event to its store. */
class Live {
  connected = $state(false)
  /** True after the connection dropped, until it is back. */
  lost = $state(false)

  #stream: EventStream | null = null
  #running = false
  #attempt = 0
  #everConnected = false
  #retryTimer: ReturnType<typeof setTimeout> | undefined

  start(): void {
    if (this.#running) return
    this.#running = true
    this.#connect()
  }

  stop(): void {
    this.#running = false
    clearTimeout(this.#retryTimer)
    this.#stream?.close()
    this.#stream = null
    this.connected = false
    this.lost = false
    this.#attempt = 0
    this.#everConnected = false
  }

  #connect(): void {
    const stream = createStream(api.eventsUrl())
    this.#stream = stream

    stream.onopen = () => {
      this.connected = true
      this.lost = false
      if (this.#everConnected) this.#resync()
      this.#everConnected = true
      this.#attempt = 0
    }

    stream.onerror = () => {
      stream.close()
      if (this.#stream !== stream) return
      this.#stream = null
      this.connected = false
      this.lost = true
      this.#scheduleReconnect()
    }

    const on = <T>(type: string, handler: (data: T) => void) =>
      stream.addEventListener(type, (event) => handler(JSON.parse(event.data || '{}') as T))

    on<StatsEvent>('stats', (data) => {
      system.pushStats(data)
      if (data.active === 0) queue.clearLive()
    })
    on<ProgressEvent>('progress', (data) => queue.applyProgress(data))
    on('queue', () => queue.invalidate())
    on('history', () => history.invalidate())
    on<LogEntry>('log', (entry) => logs.push(entry))
    on<NoticeEvent>('notice', (notice) => {
      const text = noticeText(notice)
      toasts.push(notice.level, text)
      if (notice.level === 'success' || notice.level === 'error')
        desktop.show('Rustload', text, `${notice.key}:${notice.params.name ?? ''}`)
    })
    on('resync', () => this.#resync())
  }

  async #scheduleReconnect(): Promise<void> {
    // The stream also fails when the session expired: re-check before reconnecting.
    await auth.load()
    if (!this.#running || auth.phase === 'login' || auth.phase === 'setup') return
    const delay = BACKOFF_MS[Math.min(this.#attempt, BACKOFF_MS.length - 1)]
    this.#attempt++
    clearTimeout(this.#retryTimer)
    this.#retryTimer = setTimeout(() => {
      if (this.#running && !this.#stream) this.#connect()
    }, delay)
  }

  #resync(): void {
    queue.invalidate()
    history.invalidate()
    system.load().catch(() => {})
    if (logs.loaded) logs.load()
  }
}

export const live = new Live()
