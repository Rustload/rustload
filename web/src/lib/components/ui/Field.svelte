<script lang="ts">
  import type { Snippet } from 'svelte'
  import { t } from '$lib/i18n/index.svelte'

  type Control = { id: string; describedBy: string | undefined; invalid: boolean }

  type Props = {
    label: string
    hint?: string
    error?: string | null
    optional?: boolean
    class?: string
    /** Renders the control; spread `id` / `aria-describedby` onto it. */
    children: Snippet<[Control]>
  }

  let { label, hint, error = null, optional = false, class: className = '', children }: Props = $props()

  const id = $props.id()
  const describedBy = $derived([hint && `${id}-hint`, error && `${id}-error`].filter(Boolean).join(' ') || undefined)
</script>

<div class="min-w-0 {className}">
  <label for={id} class="mb-1.5 block font-medium text-fg">
    {label}
    {#if optional}<span class="font-normal text-fg-faint">({t('common.optional')})</span>{/if}
  </label>
  {@render children({ id, describedBy, invalid: !!error })}
  {#if error}
    <p id="{id}-error" class="mt-1.5 text-small text-bad">{error}</p>
  {:else if hint}
    <p id="{id}-hint" class="mt-1.5 text-small text-fg-muted">{hint}</p>
  {/if}
</div>
