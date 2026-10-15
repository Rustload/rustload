<script lang="ts">
  import type { Snippet } from 'svelte'

  type Props = {
    /** Id of the heading, for the section's `aria-labelledby`. */
    id: string
    title: string
    count?: number
    /** A link on the right. */
    aside?: Snippet
  }

  let { id, title, count, aside }: Props = $props()
</script>

<div class="mb-2.5 flex items-baseline justify-between gap-4">
  <h2 {id} class="flex items-baseline gap-2 text-lead font-semibold">
    <span class="heading">{title}</span>
    {#if count !== undefined}<span class="num font-normal text-fg-faint">{count}</span>{/if}
  </h2>
  {@render aside?.()}
</div>
