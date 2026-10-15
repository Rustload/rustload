import { api, type Settings } from '$lib/api'
import { i18n, t } from '$lib/i18n/index.svelte'
import { kibToBytes } from '$lib/utils/format'
import { theme } from './theme.svelte'
import { errorMessage, toasts } from './toasts.svelte'

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

/**
 * Server settings plus one shared draft edited by every Settings tab, so unsaved edits survive
 * switching tabs and a single save bar covers them all.
 */
class SettingsStore {
  current = $state<Settings | null>(null)
  draft = $state<Settings | null>(null)
  saving = $state(false)
  error = $state<string | null>(null)

  dirty = $derived(
    this.current !== null && this.draft !== null && JSON.stringify(this.draft) !== JSON.stringify(this.current),
  )
  /** The saved global speed limit in bytes/s, 0 when off. */
  speedLimit = $derived.by(() => {
    const d = this.current?.downloads
    return d?.speed_limit_enabled && d.speed_limit_kib > 0 ? kibToBytes(d.speed_limit_kib) : 0
  })

  constructor() {
    // Settings may have changed from another device or through the API: refresh when the tab
    // comes back, keeping any unsaved edits.
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'visible' && this.current) this.refresh().catch(() => {})
    })
  }

  async load(): Promise<void> {
    this.#accept(await api.settings.get(), true)
  }

  async refresh(): Promise<void> {
    this.#accept(await api.settings.get(), !this.dirty)
  }

  async save(): Promise<boolean> {
    if (!this.draft) return false
    this.saving = true
    this.error = null
    try {
      this.#accept(await api.settings.save(clone(this.draft)), true)
      toasts.success(t('common.saved'))
      return true
    } catch (err) {
      this.error = errorMessage(err)
      return false
    } finally {
      this.saving = false
    }
  }

  reset(): void {
    if (this.current) this.draft = clone(this.current)
    this.error = null
  }

  /** Applies a change immediately (outside the draft), keeping any unsaved draft edits. */
  async patch(change: (settings: Settings) => void): Promise<void> {
    if (!this.current) return
    const next = clone(this.current)
    change(next)
    try {
      const saved = await api.settings.save(next)
      const keepDraft = this.dirty
      this.#accept(saved, !keepDraft)
      if (keepDraft && this.draft) change(this.draft)
    } catch (err) {
      toasts.error(err)
    }
  }

  // Theme and speed limit switches apply at once instead of going through the draft.

  setTheme(value: Settings['general']['theme']): Promise<void> {
    theme.set(value)
    return this.patch((s) => (s.general.theme = value))
  }

  toggleSpeedLimit(): Promise<void> {
    return this.patch((s) => (s.downloads.speed_limit_enabled = !s.downloads.speed_limit_enabled))
  }

  setApiKey(apiKey: string): void {
    if (this.current) this.current.integrations.api_key = apiKey
    if (this.draft) this.draft.integrations.api_key = apiKey
  }

  #accept(settings: Settings, replaceDraft: boolean): void {
    this.current = settings
    if (replaceDraft || !this.draft) this.draft = clone(settings)
    i18n.set(settings.general.language)
    theme.set(settings.general.theme)
  }
}

export const settings = new SettingsStore()
