<script lang="ts">
  import type { Snippet } from 'svelte'

  type Props = {
    title?: string
    description?: string
    actions?: Snippet
    children: Snippet
    /** Removes the inner padding, for lists that draw their own rows. */
    flush?: boolean
    /** Id for the title, which then names the panel (a landmark: use it for panels that stand alone). */
    headingId?: string
    class?: string
  }

  let { title, description, actions, children, flush = false, headingId, class: className = '' }: Props = $props()
</script>

<section aria-labelledby={headingId} class="rounded-panel border border-line bg-panel {className}">
  {#if title || actions}
    <header class="flex flex-wrap items-start justify-between gap-x-4 gap-y-2 px-5 pt-4 {flush ? 'pb-3' : ''}">
      <div class="min-w-0">
        {#if title}<h2 id={headingId} class="text-lead font-semibold heading">{title}</h2>{/if}
        {#if description}<p class="mt-0.5 max-w-[68ch] text-small text-fg-muted">{description}</p>{/if}
      </div>
      {#if actions}<div class="flex items-center gap-2">{@render actions()}</div>{/if}
    </header>
  {/if}
  <div class={flush ? '' : 'space-y-5 px-5 pt-4 pb-5'}>
    {@render children()}
  </div>
</section>
