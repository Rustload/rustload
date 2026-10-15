<script lang="ts">
  import { t } from '$lib/i18n/index.svelte'
  import { SPEED_WINDOW, system } from '$lib/stores/system.svelte'
  import { formatDuration, formatSpeed, niceBytesCeil } from '$lib/utils/format'

  /** Total speed over the last minutes, small enough to sit next to the headline. */
  let { limit = 0 }: { limit?: number } = $props()

  const samples = $derived(system.samples)
  const peak = $derived(samples.reduce((max, v) => Math.max(max, v), 0))
  const scale = $derived(niceBytesCeil(Math.max(peak, limit) * 1.08))
  // The chart spans at least a minute and grows to the full window, so a fresh session isn't a thin
  // sliver at the right edge.
  const span = $derived(Math.min(SPEED_WINDOW, Math.max(60, samples.length)))
  const offset = $derived(span - samples.length)

  /** Index into `samples` under the pointer, null when not hovering. */
  let hover = $state<number | null>(null)

  function track(event: PointerEvent) {
    const box = (event.currentTarget as HTMLElement).getBoundingClientRect()
    const slot = Math.floor(((event.clientX - box.left) / box.width) * span) - offset
    hover = slot >= 0 && slot < samples.length ? slot : null
  }

  const readout = $derived.by(() => {
    if (hover === null || hover >= samples.length) return null
    const ago = samples.length - 1 - hover
    const speed = formatSpeed(samples[hover])
    return ago === 0 ? t('dashboard.speedNow', { speed }) : t('dashboard.speedAgo', { ago: formatDuration(ago), speed })
  })
</script>

<figure class="min-w-0">
  <div
    class="relative h-10 border-b border-line-strong sm:h-16"
    role="img"
    aria-label="{t('dashboard.speedCaption')}, {t('dashboard.peak', { speed: formatSpeed(peak) })}"
    onpointermove={track}
    onpointerleave={() => (hover = null)}
  >
    <svg
      viewBox="0 0 {span} 100"
      preserveAspectRatio="none"
      class="absolute inset-0 size-full overflow-visible"
      aria-hidden="true"
    >
      {#each samples as value, i (i)}
        {#if value > 0}
          {@const h = Math.max(2, (value / scale) * 100)}
          <rect
            x={offset + i + 0.12}
            y={100 - h}
            width="0.76"
            height={h}
            fill="var(--signal)"
            opacity={i === hover || (hover === null && i === samples.length - 1) ? 1 : 0.45}
          />
        {/if}
      {/each}
      {#if limit > 0}
        <line
          x1="0"
          x2={span}
          y1={100 - (limit / scale) * 100}
          y2={100 - (limit / scale) * 100}
          stroke="var(--fg-muted)"
          stroke-width="1"
          stroke-dasharray="4 3"
          vector-effect="non-scaling-stroke"
        />
      {/if}
    </svg>
  </div>
  <figcaption class="mt-1.5 flex justify-between gap-4 num text-xs text-fg-faint">
    {#if readout}
      <span class="text-fg-muted">{readout}</span>
    {:else}
      <span>{t('dashboard.speedCaption')}</span>
      <span>
        {#if limit > 0}{t('dashboard.limitLine', { speed: formatSpeed(limit) })},{/if}
        {t('dashboard.peak', { speed: formatSpeed(peak) })}
      </span>
    {/if}
  </figcaption>
</figure>
