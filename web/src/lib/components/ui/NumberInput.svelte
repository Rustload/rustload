<script lang="ts">
  import { untrack } from 'svelte'
  import { inputClass } from './styles'

  type Props = {
    value: number
    min?: number
    max?: number
    step?: number
    suffix?: string
    id?: string
    describedBy?: string
    disabled?: boolean
    class?: string
  }

  let {
    value = $bindable(),
    min,
    max,
    step = 1,
    suffix,
    id,
    describedBy,
    disabled = false,
    class: className = '',
  }: Props = $props()

  // Edit a text buffer so intermediate states ("", "1.") don't fight the bound number.
  let text = $state('')
  $effect.pre(() => {
    const next = value
    untrack(() => {
      if (text.trim() === '' || Number(text.replace(',', '.')) !== next) text = String(next)
    })
  })

  function clamp(n: number): number {
    if (min !== undefined) n = Math.max(min, n)
    if (max !== undefined) n = Math.min(max, n)
    return n
  }

  function oninput() {
    const parsed = Number(text.replace(',', '.'))
    if (text.trim() !== '' && Number.isFinite(parsed)) value = clamp(parsed)
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return
    event.preventDefault()
    value = clamp(value + (event.key === 'ArrowUp' ? step : -step))
    text = String(value)
  }
</script>

<div class="relative {className}">
  <input
    {id}
    type="text"
    inputmode="decimal"
    autocomplete="off"
    role="spinbutton"
    aria-valuenow={value}
    aria-valuemin={min}
    aria-valuemax={max}
    aria-describedby={describedBy}
    {disabled}
    bind:value={text}
    {oninput}
    {onkeydown}
    onblur={() => (text = String(value))}
    class="{inputClass} num text-lead {suffix ? 'pr-16' : ''}"
  />
  {#if suffix}
    <span class="pointer-events-none absolute inset-y-0 right-3 flex items-center text-small text-fg-faint">
      {suffix}
    </span>
  {/if}
</div>
