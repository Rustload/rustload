<script lang="ts">
  import Search from '@lucide/svelte/icons/search'
  import type { HTMLInputAttributes } from 'svelte/elements'
  import { inputClass } from './styles'

  type Props = Omit<HTMLInputAttributes, 'type' | 'value'> & {
    value: string
    /** Accessible name; defaults to the placeholder. */
    label?: string
  }

  let { value = $bindable(), label, placeholder, class: className = '', ...rest }: Props = $props()
</script>

<div class="relative {className}">
  <Search
    size={15}
    class="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-fg-faint"
    aria-hidden="true"
  />
  <input type="search" bind:value {placeholder} aria-label={label ?? placeholder} class="{inputClass} pl-9" {...rest} />
</div>
