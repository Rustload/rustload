<script lang="ts">
  import { clock } from '$lib/stores/ui.svelte'

  /** A 24-hour bar highlighting the `start`–`end` window (crossing midnight when `end` < `start`). */
  type Props = { start: string; end: string; label: string; tone?: 'signal' | 'muted'; compact?: boolean }

  let { start, end, label, tone = 'signal', compact = false }: Props = $props()

  function minutes(hhmm: string): number {
    const [h, m] = hhmm.split(':').map(Number)
    return (h || 0) * 60 + (m || 0)
  }

  const from = $derived(minutes(start))
  const to = $derived(minutes(end))

  /** Highlighted spans as [left %, width %]. */
  const spans = $derived.by((): [number, number][] => {
    const pct = (m: number) => (m / 1440) * 100
    if (from === to) return [[0, 100]]
    if (to > from) return [[pct(from), pct(to - from)]]
    return [
      [pct(from), 100 - pct(from)],
      [0, pct(to)],
    ]
  })

  const nowPct = $derived.by(() => {
    const d = new Date(clock.now)
    return ((d.getHours() * 60 + d.getMinutes()) / 1440) * 100
  })
</script>

<div
  class="relative overflow-hidden rounded-control border border-line bg-sunken {compact ? 'h-3' : 'h-9'}"
  role="img"
  aria-label={label}
>
  {#each spans as [left, width], i (i)}
    <div
      class="absolute inset-y-0 border-x {tone === 'signal'
        ? 'border-signal/60 bg-signal-wash'
        : 'border-fg-faint/60 bg-fg-faint/25'}"
      style:left="{left}%"
      style:width="{width}%"
    ></div>
  {/each}
  {#each [6, 12, 18] as hour (hour)}
    <div class="absolute inset-y-0 border-l border-dashed border-line" style:left="{(hour / 24) * 100}%"></div>
  {/each}
  <div class="absolute inset-y-0 w-0.5 bg-fg" style:left="{nowPct}%" aria-hidden="true"></div>
</div>
