import { api, type Package } from '$lib/api'
import { runWithToast, toasts } from './toasts.svelte'

export const HISTORY_PAGE_SIZE = 25

class History {
  items = $state<Package[]>([])
  total = $state(0)
  query = $state('')
  page = $state(0)
  loading = $state(false)
  loaded = $state(false)

  pages = $derived(Math.max(1, Math.ceil(this.total / HISTORY_PAGE_SIZE)))
  /** Bumped on every change, so other views (the dashboard) know when to refetch. */
  version = $state(0)

  /** Number of mounted views; invalidations only refetch while someone is looking. */
  #viewers = 0
  #stale = true
  #timer: ReturnType<typeof setTimeout> | undefined
  #requestId = 0

  async load(): Promise<void> {
    const requestId = ++this.#requestId
    this.loading = true
    try {
      const { items, total } = await api.history.list(
        this.query.trim(),
        HISTORY_PAGE_SIZE,
        this.page * HISTORY_PAGE_SIZE,
      )
      if (requestId !== this.#requestId) return
      this.items = items
      this.total = total
      this.loaded = true
      this.#stale = false
      if (this.page > 0 && this.page >= this.pages) {
        this.page = this.pages - 1
        await this.load()
      }
    } catch (err) {
      if (requestId === this.#requestId) toasts.error(err)
    } finally {
      if (requestId === this.#requestId) this.loading = false
    }
  }

  invalidate(delay = 200): void {
    this.version++
    this.#stale = true
    if (this.#viewers === 0) return
    clearTimeout(this.#timer)
    this.#timer = setTimeout(() => this.load(), delay)
  }

  /** Registers a mounted view; returns the cleanup for `$effect`. */
  attach(): () => void {
    this.#viewers++
    if (this.#stale) this.load()
    return () => {
      this.#viewers--
    }
  }

  search(query: string): void {
    this.query = query
    this.page = 0
    this.invalidate(250)
  }

  goTo(page: number): void {
    this.page = Math.min(Math.max(0, page), this.pages - 1)
    this.load()
  }

  async run(action: () => Promise<unknown>): Promise<void> {
    if (await runWithToast(action)) this.invalidate(0)
  }
}

export const history = new History()
