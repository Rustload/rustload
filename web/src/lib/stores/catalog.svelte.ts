import { api, type Account, type Category, type CategoryInput, type Provider } from '$lib/api'

const PLUGIN_NAMES: Record<string, string> = {
  mega: 'MEGA',
  gdrive: 'Google Drive',
  dropbox: 'Dropbox',
  mediafire: 'MediaFire',
  pixeldrain: 'Pixeldrain',
  gofile: 'Gofile',
  media: 'yt-dlp',
  'yt-dlp': 'yt-dlp',
}

/** `list` with the item at `index` moved `delta` places, for "move up / down" menus. */
function moved<T>(list: T[], index: number, delta: number): T[] {
  const next = [...list]
  const [item] = next.splice(index, 1)
  next.splice(index + delta, 0, item)
  return next
}

/** Destination categories (folders) and the download root. */
class Categories {
  list = $state<Category[]>([])
  root = $state('')
  defaultCategory = $derived(this.list.find((c) => c.is_default) ?? null)

  async load(): Promise<void> {
    const { categories, download_root } = await api.categories.list()
    this.list = categories
    this.root = download_root
  }

  byId(id: number | null): Category | null {
    return id === null ? null : (this.list.find((c) => c.id === id) ?? null)
  }

  async save(id: number | null, input: CategoryInput): Promise<void> {
    if (id === null) await api.categories.create(input)
    else await api.categories.update(id, input)
    await this.load()
  }

  async remove(id: number): Promise<void> {
    await api.categories.remove(id)
    await this.load()
  }

  /** Moves a category up or down at once, then saves the order. */
  async move(index: number, delta: number): Promise<void> {
    this.list = moved(this.list, index, delta)
    await api.categories.reorder(this.list.map((c) => c.id))
    await this.load()
  }
}

/** Debrid accounts and the providers the server supports. */
class Accounts {
  list = $state<Account[]>([])
  providers = $state<Provider[]>([])

  /** True when at least one enabled account can process magnets/torrents. */
  magnetCapable = $derived(this.list.some((a) => a.enabled && this.provider(a.provider)?.magnets))

  async load(): Promise<void> {
    const { accounts, providers } = await api.accounts.list()
    this.list = accounts
    this.providers = providers
  }

  provider(id: string): Provider | undefined {
    return this.providers.find((p) => p.id === id)
  }

  /** Display name of whatever fetched a file: an account provider, a built-in plugin or yt-dlp. */
  providerName(id: string): string {
    return this.provider(id)?.name ?? PLUGIN_NAMES[id] ?? id
  }

  replace(account: Account): void {
    const index = this.list.findIndex((a) => a.id === account.id)
    if (index === -1) this.list = [...this.list, account]
    else this.list[index] = account
  }

  async remove(id: number): Promise<void> {
    await api.accounts.remove(id)
    this.list = this.list.filter((a) => a.id !== id)
  }

  /** Moves an account up or down at once (accounts are tried in order), then saves the order. */
  async move(index: number, delta: number): Promise<void> {
    this.list = moved(this.list, index, delta)
    await api.accounts.reorder(this.list.map((a) => a.id))
  }
}

export const categories = new Categories()
export const accounts = new Accounts()
