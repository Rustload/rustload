import { api, type LogEntry } from '$lib/api'
import { toasts } from './toasts.svelte'

const MAX_ENTRIES = 2000
const FLUSH_MS = 200

/** Ring buffer of log entries: initial fetch + live `log` events, batched to limit re-renders. */
class Logs {
  entries = $state.raw<LogEntry[]>([])
  loaded = $state(false)

  #pending: LogEntry[] = []
  #flushTimer: ReturnType<typeof setTimeout> | undefined

  async load(): Promise<void> {
    try {
      const { entries } = await api.logs.list(1000)
      this.entries = entries
      this.loaded = true
    } catch (err) {
      toasts.error(err)
    }
  }

  push(entry: LogEntry): void {
    this.#pending.push(entry)
    this.#flushTimer ??= setTimeout(() => this.#flush(), FLUSH_MS)
  }

  async clear(): Promise<void> {
    try {
      await api.logs.clear()
      this.entries = []
      this.#pending = []
    } catch (err) {
      toasts.error(err)
    }
  }

  #flush(): void {
    this.#flushTimer = undefined
    const lastId = this.entries.at(-1)?.id ?? -1
    const fresh = this.#pending.filter((entry) => entry.id > lastId)
    this.#pending = []
    if (fresh.length === 0) return
    const merged = this.entries.concat(fresh)
    this.entries = merged.length > MAX_ENTRIES ? merged.slice(-MAX_ENTRIES) : merged
  }
}

export const logs = new Logs()
