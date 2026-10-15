<script lang="ts">
  type Row = { key: string; label: string; value: number; detail?: string }

  /** Ranked horizontal bars with the value at the tip: hosts, providers… */
  let { rows, format, label }: { rows: Row[]; format: (value: number) => string; label: string } = $props()

  const max = $derived(Math.max(1, ...rows.map((r) => r.value)))
</script>

<ul class="space-y-3" aria-label={label}>
  {#each rows as row (row.key)}
    <li>
      <div class="flex items-baseline justify-between gap-3 text-small">
        <span class="min-w-0 truncate">{row.label}</span>
        <span class="shrink-0 num text-fg-muted">
          {format(row.value)}{#if row.detail}<span class="ml-2 text-fg-faint">{row.detail}</span>{/if}
        </span>
      </div>
      <div class="mt-1.5 h-1.5 rounded-full bg-sunken" aria-hidden="true">
        <div class="h-full rounded-full bg-fg-faint" style:width="{Math.max(1.5, (row.value / max) * 100)}%"></div>
      </div>
    </li>
  {/each}
</ul>
