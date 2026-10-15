<script lang="ts">
  /**
   * The file seen as 100 slices: how much of each is on disk, and where every connection is
   * writing right now. Makes the multi-connection engine visible.
   */
  type Props = {
    /** 100 digits, `0`–`9` (9 = slice complete). */
    map: string
    /** Write position of each connection, 0..1. */
    heads: number[]
    label: string
    size?: 'sm' | 'md'
  }

  let { map, heads, label, size = 'sm' }: Props = $props()

  // Consecutive slices with the same level are drawn as one run to keep the DOM small.
  const runs = $derived.by(() => {
    const list: { start: number; length: number; level: number }[] = []
    for (let i = 0; i < map.length; i++) {
      const level = Number(map[i]) || 0
      const last = list[list.length - 1]
      if (last && last.level === level) last.length++
      else list.push({ start: i, length: 1, level })
    }
    return list.filter((run) => run.level > 0)
  })
  const percent = $derived(
    Math.round(([...map].reduce((sum, c) => sum + (Number(c) || 0), 0) / (9 * map.length || 1)) * 100),
  )
</script>

<div
  class="relative w-full overflow-hidden rounded-full bg-sunken {size === 'md' ? 'h-2' : 'h-1.5'}"
  role="progressbar"
  aria-label={label}
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={percent}
  title={label}
>
  {#each runs as run (run.start)}
    <div
      class="absolute inset-y-0 bg-signal transition-opacity duration-700"
      style:left="{run.start}%"
      style:width="{run.length}%"
      style:opacity={run.level === 9 ? 1 : 0.2 + (run.level / 9) * 0.5}
    ></div>
  {/each}
  {#each heads as head, i (i)}
    <div
      class="head absolute inset-y-0 w-0.5 -translate-x-1/2 bg-fg transition-[left] duration-1000 ease-linear"
      style:left="{head * 100}%"
    ></div>
  {/each}
</div>

<style>
  .head {
    box-shadow: 0 0 4px var(--signal);
  }
</style>
