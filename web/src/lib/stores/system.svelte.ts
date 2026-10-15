import { api, type StatsEvent, type System } from '$lib/api'
import { toasts } from './toasts.svelte'

/** Number of one-second speed samples kept for the dashboard chart. */
export const SPEED_WINDOW = 180

class SystemStore {
  info = $state<System | null>(null)
  /** Latest `stats` event; null until the event stream delivers one. */
  stats = $state.raw<StatsEvent | null>(null)
  /** Total speed over the last SPEED_WINDOW seconds, oldest first. */
  samples = $state.raw<number[]>([])

  paused = $derived(this.stats?.paused ?? this.info?.paused ?? false)
  scheduleOpen = $derived(this.stats?.schedule_open ?? this.info?.schedule_open ?? true)
  /** Free space under the configured minimum: downloads are on hold. */
  diskLow = $derived(this.stats?.disk_low ?? this.info?.disk_low ?? false)
  speed = $derived(this.stats?.speed ?? 0)

  async load(): Promise<void> {
    this.info = await api.system.get()
  }

  pushStats(event: StatsEvent): void {
    this.stats = event
    const next = this.samples.length >= SPEED_WINDOW ? this.samples.slice(1) : this.samples.slice()
    next.push(event.speed)
    this.samples = next
  }

  async togglePause(): Promise<void> {
    try {
      this.info = this.paused ? await api.system.resume() : await api.system.pause()
      if (this.stats) this.stats = { ...this.stats, paused: this.info.paused }
    } catch (err) {
      toasts.error(err)
    }
  }
}

export const system = new SystemStore()
