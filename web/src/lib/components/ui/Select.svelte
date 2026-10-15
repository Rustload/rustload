<script lang="ts" generics="T">
  import ChevronDown from '@lucide/svelte/icons/chevron-down'
  import { inputClass } from './styles'

  type Option = { value: T; label: string; disabled?: boolean }

  type Props = {
    value: T
    options: Option[]
    id?: string
    describedBy?: string
    disabled?: boolean
    ariaLabel?: string
    onchange?: (value: T) => void
    class?: string
  }

  let {
    value = $bindable(),
    options,
    id,
    describedBy,
    disabled = false,
    ariaLabel,
    onchange,
    class: className = '',
  }: Props = $props()
</script>

<div class="relative {className}">
  <select
    {id}
    {disabled}
    aria-label={ariaLabel}
    aria-describedby={describedBy}
    bind:value
    onchange={() => onchange?.(value)}
    class="{inputClass} cursor-pointer appearance-none pr-9"
  >
    {#each options as option (option.value)}
      <option value={option.value} disabled={option.disabled}>{option.label}</option>
    {/each}
  </select>
  <ChevronDown
    size={16}
    class="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2 text-fg-faint"
    aria-hidden="true"
  />
</div>
