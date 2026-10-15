<script lang="ts" module>
  export type Column = {
    /** Axis label (may be shown sparsely). */
    label: string
    /** Tooltip heading; defaults to the axis label. */
    title?: string
    value: number
    /** Lines shown in the tooltip under the label. */
    details: string[]
    /** Drawn in the signal colour: the column that is still moving (today, this hour). */
    live?: boolean
  }
</script>

<script lang="ts">
  type Props = {
    columns: Column[]
    format: (value: number) => string
    /** Rounds the scale maximum up to a clean value. */
    niceMax: (value: number) => number
    label: string
    height?: number
    /** Show every n-th axis label (the first and last are always shown). */
    labelEvery?: number
  }

  let { columns, format, niceMax, label, height = 180, labelEvery = 1 }: Props = $props()

  let width = $state(0)
  let active = $state<number | null>(null)

  const AXIS = 22
  const GUTTER = 64
  const plotHeight = $derived(height - AXIS)
  const plotWidth = $derived(Math.max(0, width - GUTTER))
  const max = $derived(niceMax(Math.max(0, ...columns.map((c) => c.value))))
  const slot = $derived(columns.length ? plotWidth / columns.length : 0)
  const barWidth = $derived(Math.max(1, Math.min(24, slot - 2)))

  function y(value: number): number {
    return max > 0 ? plotHeight - (value / max) * plotHeight : plotHeight
  }

  /** A column with a 4 px rounded top and a square base. */
  function columnPath(x: number, value: number): string {
    const top = y(value)
    const h = plotHeight - top
    if (h <= 0) return ''
    const r = Math.min(4, barWidth / 2, h)
    return `M${x},${plotHeight}V${top + r}Q${x},${top} ${x + r},${top}H${x + barWidth - r}Q${x + barWidth},${top} ${x + barWidth},${top + r}V${plotHeight}Z`
  }

  function showLabel(i: number): boolean {
    return i === 0 || i === columns.length - 1 || (i % labelEvery === 0 && columns.length - 1 - i >= labelEvery / 2)
  }

  function onpointermove(event: PointerEvent) {
    const rect = (event.currentTarget as SVGElement).getBoundingClientRect()
    const index = Math.floor((event.clientX - rect.left) / slot)
    active = index >= 0 && index < columns.length ? index : null
  }

  const tooltip = $derived(active !== null ? columns[active] : null)
  const tooltipLeft = $derived(active !== null ? Math.min(Math.max(active * slot + slot / 2, 70), plotWidth - 70) : 0)
</script>

<div class="relative" bind:clientWidth={width}>
  <!-- Hover details only enhance: every value is also in the table view of the page. -->
  <svg {width} {height} role="img" aria-label={label} class="block overflow-visible">
    {#each [0, 0.5, 1] as fraction (fraction)}
      {@const lineY = y(max * fraction)}
      <line
        x1="0"
        x2={plotWidth}
        y1={lineY}
        y2={lineY}
        stroke="var(--line)"
        stroke-width="1"
        shape-rendering="crispEdges"
      />
      <text x={plotWidth + 8} y={lineY} dy="0.32em" class="fill-fg-faint num text-micro">{format(max * fraction)}</text>
    {/each}

    {#each columns as column, i (i)}
      {@const x = i * slot + (slot - barWidth) / 2}
      <path
        d={columnPath(x, column.value)}
        fill={column.live ? 'var(--signal)' : active === i ? 'var(--fg)' : 'var(--fg-faint)'}
        opacity={active === null || active === i ? 1 : 0.55}
      />
      {#if showLabel(i)}
        <text
          x={i * slot + slot / 2}
          y={height - 4}
          text-anchor={i === 0 ? 'start' : i === columns.length - 1 ? 'end' : 'middle'}
          dx={i === 0 ? -(slot - barWidth) / 2 : i === columns.length - 1 ? (slot - barWidth) / 2 : 0}
          class="fill-fg-faint num text-micro">{column.label}</text
        >
      {/if}
    {/each}

    <!-- Hit layer: whole column slots, wider than the marks. -->
    <rect
      x="0"
      y="0"
      width={plotWidth}
      height={plotHeight}
      fill="transparent"
      role="presentation"
      {onpointermove}
      onpointerleave={() => (active = null)}
    />
  </svg>

  {#if tooltip}
    <div
      class="pointer-events-none absolute top-0 z-10 -translate-x-1/2 -translate-y-[calc(100%+6px)] rounded-control border
        border-line bg-raised px-3 py-2 text-small whitespace-nowrap shadow-pop"
      style:left="{tooltipLeft}px"
      role="status"
    >
      <p class="font-medium">{tooltip.title ?? tooltip.label}</p>
      <p class="num">{format(tooltip.value)}</p>
      {#each tooltip.details as line, i (i)}
        <p class="num text-xs text-fg-muted">{line}</p>
      {/each}
    </div>
  {/if}
</div>
