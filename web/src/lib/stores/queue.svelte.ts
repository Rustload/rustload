import { api, type Download, type MoveTarget, type Package, type ProgressEvent } from '$lib/api'
import { isActiveDownload } from '$lib/utils/status'
import { runWithToast } from './toasts.svelte'

/**
 * The queue (unfinished packages). Live `progress` events are merged in place into the reactive
 * objects so only the rows whose values changed re-render.
 */
class Queue {
  packages = $state<Package[]>([])
  loaded = $state(false)

  /** Files currently transferring or resolving, in queue order. */
  active = $derived(this.packages.flatMap((p) => p.files.filter((f) => isActiveDownload(f.status))))
  remote = $derived(this.packages.filter((p) => p.status === 'debrid'))
  /** Packages being verified or extracted once their files are downloaded. */
  postprocessing = $derived(this.packages.filter((p) => p.status === 'extracting' || p.status === 'verifying'))

  #downloads = new Map<number, Download>()
  #packages = new Map<number, Package>()
  #live = new Set<number>()
  #refreshTimer: ReturnType<typeof setTimeout> | undefined
  #loading: Promise<void> | null = null

  async load(): Promise<void> {
    if (this.#loading) return this.#loading
    this.#loading = (async () => {
      try {
        const { packages } = await api.packages.list()
        this.packages = packages
        this.#reindex()
        this.loaded = true
      } finally {
        this.#loading = null
      }
    })()
    return this.#loading
  }

  /** Refetches soon; several invalidations in a burst cause a single request. */
  invalidate(): void {
    clearTimeout(this.#refreshTimer)
    this.#refreshTimer = setTimeout(() => {
      this.load().catch(() => {})
    }, 150)
  }

  applyProgress(event: ProgressEvent): void {
    let unknown = false
    const seen = new Set<number>()

    for (const p of event.downloads) {
      const file = this.#downloads.get(p.id)
      if (!file) {
        unknown = true
        continue
      }
      seen.add(p.id)
      file.status = p.status
      file.downloaded = p.downloaded
      file.size = p.size
      file.speed = p.speed
      file.eta = p.eta
      file.connections = p.connections
      file.map = p.map
      file.heads = p.heads
    }
    this.#resetLive((id) => !seen.has(id))
    this.#live = seen

    for (const p of event.packages) {
      const pkg = this.#packages.get(p.id)
      if (!pkg) {
        unknown = true
        continue
      }
      pkg.status = p.status
      pkg.debrid = p.debrid
      pkg.extract_progress = p.extract_progress
      pkg.verify_progress = p.verify_progress
    }

    if (unknown) this.invalidate()
  }

  /** Called when the server reports no activity: zero every live value we still display. */
  clearLive(): void {
    if (this.#live.size === 0) return
    this.#resetLive(() => true)
    this.#live.clear()
  }

  package(id: number): Package | undefined {
    return this.#packages.get(id)
  }

  // ---- actions (the server also emits a `queue` event; refetching here keeps the UI snappy) ----

  async run(action: () => Promise<unknown>): Promise<boolean> {
    const ok = await runWithToast(action)
    if (ok) this.invalidate()
    return ok
  }

  move(id: number, to: MoveTarget): Promise<boolean> {
    return this.run(() => api.packages.move(id, to))
  }

  /** Optimistically applies a new package order, then persists it. */
  async reorder(ids: number[]): Promise<void> {
    const byId = new Map(this.packages.map((p) => [p.id, p]))
    const ordered = ids.map((id) => byId.get(id)).filter((p): p is Package => !!p)
    const rest = this.packages.filter((p) => !ids.includes(p.id))
    this.packages = [...ordered, ...rest]
    this.#reindex()
    await this.run(() => api.packages.reorder(this.packages.map((p) => p.id)))
  }

  #reindex(): void {
    this.#downloads.clear()
    this.#packages.clear()
    for (const pkg of this.packages) {
      this.#packages.set(pkg.id, pkg)
      for (const file of pkg.files) this.#downloads.set(file.id, file)
    }
  }

  #resetLive(predicate: (id: number) => boolean): void {
    for (const id of this.#live) {
      if (!predicate(id)) continue
      const file = this.#downloads.get(id)
      if (!file) continue
      file.speed = 0
      file.eta = null
      file.connections = 0
      file.map = null
      file.heads = []
    }
  }
}

export const queue = new Queue()
