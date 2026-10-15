<script lang="ts">
  import { t, tSplit } from '$lib/i18n/index.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { system } from '$lib/stores/system.svelte'
  import { clock } from '$lib/stores/ui.svelte'
  import { finishAt, formatBytes, formatSpeed, splitSpeed } from '$lib/utils/format'
  import SpeedSpark from './SpeedSpark.svelte'

  /** One sentence that says what Rustload is doing, with the figures that matter right under it. */

  const stats = $derived(system.stats)
  const active = $derived(stats?.active ?? queue.active.length)
  const queued = $derived(stats?.queued ?? 0)
  const remaining = $derived(formatBytes(stats?.queue_remaining ?? 0))

  const state = $derived.by(() => {
    if (system.paused) return 'paused'
    if (active > 0 || system.speed > 0) return 'downloading'
    if (queue.remote.length) return 'remote'
    if (queue.postprocessing.length) return 'processing'
    if (queued > 0) return system.diskLow ? 'diskLow' : system.scheduleOpen ? 'waiting' : 'scheduled'
    return 'idle'
  })

  // The limit in force (global limit or speed rule); the settings stand in until the first stats event.
  const limit = $derived(stats ? stats.speed_limit : settings.speedLimit)

  const readout = $derived(splitSpeed(system.speed))
  const headline = $derived(tSplit('dashboard.headline.downloading', 'speed'))

  const finish = $derived.by(() => {
    const end = finishAt(stats?.eta, clock.now, 'weekday')
    if (!end) return t('dashboard.summary.remaining', { size: remaining })
    return end.day === null
      ? t('dashboard.summary.finishToday', { time: end.time, size: remaining })
      : t('dashboard.summary.finishLater', { day: end.day, time: end.time, size: remaining })
  })

  const summary = $derived.by(() => {
    switch (state) {
      case 'downloading': {
        const parts = [t('dashboard.summary.running', { count: active }), finish]
        if (limit > 0) {
          const key = stats?.speed_rule_active ? 'dashboard.summary.limitedByRule' : 'dashboard.summary.limited'
          parts.push(t(key, { speed: formatSpeed(limit) }))
        }
        return parts.join(' ')
      }
      case 'paused':
        return queued > 0
          ? t('dashboard.summary.waiting', { count: queued, size: remaining })
          : t('dashboard.summary.nothingWaiting')
      case 'remote':
        return t('dashboard.summary.remote', { count: queue.remote.length })
      case 'processing':
        return t('dashboard.summary.processing')
      case 'diskLow':
        return t('dashboard.diskLow')
      case 'scheduled':
        return t('dashboard.summary.scheduled')
      case 'waiting':
        return t('dashboard.summary.waiting', { count: queued, size: remaining })
      default:
        return t('dashboard.summary.idle')
    }
  })

  const showSpark = $derived(system.samples.some((v) => v > 0))
</script>

<header
  class="grid items-end gap-x-12 gap-y-5 border-b border-line pb-6 lg:grid-cols-[minmax(0,1fr)_minmax(16rem,24rem)]"
>
  <div class="min-w-0">
    <h2 class="heading text-[1.5rem] leading-[1.15] font-semibold text-balance sm:text-[2rem]">
      {#if state === 'downloading'}
        {headline[0]}<span class="whitespace-nowrap"
          ><span class="readout text-[1.15em] font-medium">{readout.value}</span>&nbsp;<span
            class="font-medium text-fg-muted">{readout.unit}</span
          ></span
        >{headline[1]}
      {:else}
        {t(`dashboard.headline.${state}`)}
      {/if}
    </h2>
    <p class="mt-2 max-w-[68ch] text-lead text-fg-muted">{summary}</p>
  </div>

  {#if showSpark}
    <SpeedSpark {limit} />
  {/if}
</header>
