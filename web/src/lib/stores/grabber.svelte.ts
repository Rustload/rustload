import { SvelteMap } from 'svelte/reactivity'
import { ApiError, api, type LinkInfo } from '$lib/api'
import { parseLinks } from '$lib/utils/links'

/** Links per `POST /api/links/check` request and requests in flight. */
const BATCH = 8
const PARALLEL = 3

export type CheckState =
  | { state: 'checking' }
  | { state: 'done'; info: LinkInfo }
  /** The check itself failed (server unreachable…): the link can still be added. */
  | { state: 'failed' }

/** What a page scan found; `stale` when the dialog was reset while the page was being read. */
export type ScanResult = { stale: true } | { stale: false; found: string[] | null }

/**
 * Live availability checks for the link grabber. Results are cached per URL for the life of the
 * dialog, so editing the pasted text never re-checks links already seen; failed checks are tried
 * again the next time the text changes.
 */
class Grabber {
  results = new SvelteMap<string, CheckState>()
  /** URLs being scanned for links (web pages). */
  scanning = new SvelteMap<string, true>()

  #pending: string[] = []
  #inFlight = 0
  #generation = 0

  reset(): void {
    this.#generation++
    this.#pending = []
    this.#inFlight = 0
    this.results.clear()
    this.scanning.clear()
  }

  /** Queues every link of `text` that hasn't been checked yet. */
  check(text: string): void {
    const { links, magnets } = parseLinks(text)
    for (const url of [...links, ...magnets]) {
      const known = this.results.get(url)
      if (known && known.state !== 'failed') continue
      this.results.set(url, { state: 'checking' })
      this.#pending.push(url)
    }
    this.#pump()
  }

  /** Forgets links, e.g. once they have been added, so checking them again flags duplicates. */
  forget(urls: Iterable<string>): void {
    for (const url of urls) this.results.delete(url)
  }

  /** Fetches a web page and returns the downloadable links found on it. */
  async scan(url: string): Promise<ScanResult> {
    const generation = this.#generation
    this.scanning.set(url, true)
    try {
      const { results } = await api.links.check([url], true)
      if (generation !== this.#generation) return { stale: true }
      const info = results[0]
      if (info) this.results.set(url, { state: 'done', info })
      return { stale: false, found: info?.found ?? null }
    } finally {
      if (generation === this.#generation) this.scanning.delete(url)
    }
  }

  #pump(): void {
    while (this.#inFlight < PARALLEL && this.#pending.length) {
      const batch = this.#pending.splice(0, BATCH)
      this.#inFlight++
      this.#run(batch, this.#generation)
    }
  }

  async #run(urls: string[], generation: number): Promise<void> {
    const infos = await this.#request(urls)
    // reset() started over while this request was running: its results are stale.
    if (generation !== this.#generation) return
    const byUrl = new Map(infos.map((info) => [info.url, info]))
    for (const url of urls) {
      const info = byUrl.get(url)
      this.results.set(url, info ? { state: 'done', info } : { state: 'failed' })
    }
    this.#inFlight--
    this.#pump()
  }

  /** One request; when the server rejects the batch (one link it can't parse), each link alone. */
  async #request(urls: string[]): Promise<LinkInfo[]> {
    try {
      return (await api.links.check(urls)).results
    } catch (err) {
      if (!(err instanceof ApiError) || err.status !== 400 || urls.length === 1) return []
      const single = await Promise.all(urls.map((url) => this.#request([url])))
      return single.flat()
    }
  }
}

export const grabber = new Grabber()
