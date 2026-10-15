<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements'
  import { inputClass } from './styles'

  type Props = Omit<HTMLInputAttributes, 'value'> & {
    value: string
    /** Unit or hint rendered inside the field, on the right. */
    suffix?: string
    invalid?: boolean
    mono?: boolean
    input?: HTMLInputElement
  }

  let {
    value = $bindable(),
    input = $bindable(),
    suffix,
    invalid = false,
    mono = false,
    type = 'text',
    class: className = '',
    ...rest
  }: Props = $props()
</script>

<div class="relative {className}">
  <input
    bind:this={input}
    bind:value
    {type}
    aria-invalid={invalid || undefined}
    class="{inputClass} {suffix ? 'pr-14' : ''} {mono ? 'font-mono text-small' : ''} {invalid ? 'border-bad' : ''}"
    {...rest}
  />
  {#if suffix}
    <span class="pointer-events-none absolute inset-y-0 right-3 flex items-center text-small text-fg-faint">
      {suffix}
    </span>
  {/if}
</div>
