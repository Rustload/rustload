<script lang="ts">
  import type { Snippet } from 'svelte'

  type Props = {
    title: string
    /** A line under the title, usually how many items the page lists. */
    count?: string | null
    actions?: Snippet
  }

  let { title, count, actions }: Props = $props()
</script>

<div class="mb-5 flex flex-wrap items-end justify-between gap-3">
  <div>
    <h1 class="text-2xl font-semibold heading">{title}</h1>
    {#if count}<p class="mt-0.5 num text-fg-muted">{count}</p>{/if}
  </div>
  {@render actions?.()}
</div>
