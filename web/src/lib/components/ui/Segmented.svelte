<script lang="ts" generics="T extends string">
  import type { LucideIcon } from '@lucide/svelte'

  type Option = { value: T; label: string; icon?: LucideIcon }

  type Props = {
    value: T
    options: Option[]
    label: string
    onchange?: (value: T) => void
    class?: string
  }

  let { value = $bindable(), options, label, onchange, class: className = '' }: Props = $props()

  function choose(next: T) {
    if (next === value) return
    value = next
    onchange?.(next)
  }

  function onkeydown(event: KeyboardEvent) {
    const step =
      event.key === 'ArrowRight' || event.key === 'ArrowDown'
        ? 1
        : event.key === 'ArrowLeft' || event.key === 'ArrowUp'
          ? -1
          : 0
    if (!step) return
    event.preventDefault()
    const index = options.findIndex((o) => o.value === value)
    const next = options[(index + step + options.length) % options.length]
    choose(next.value)
    ;(event.currentTarget as HTMLElement).querySelector<HTMLElement>(`[data-value="${next.value}"]`)?.focus()
  }
</script>

<div
  role="radiogroup"
  aria-label={label}
  tabindex="-1"
  {onkeydown}
  class="inline-flex rounded-control border border-line bg-sunken p-0.5 {className}"
>
  {#each options as option (option.value)}
    {@const selected = option.value === value}
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      tabindex={selected ? 0 : -1}
      data-value={option.value}
      onclick={() => choose(option.value)}
      class="inline-flex h-7 items-center gap-1.5 rounded-chip px-2.5 text-small font-medium transition-colors
        {selected ? 'bg-raised text-fg shadow-[0_1px_0_var(--line)]' : 'text-fg-muted hover:text-fg'}"
    >
      {#if option.icon}
        <option.icon size={14} aria-hidden="true" />
      {/if}
      {option.label}
    </button>
  {/each}
</div>
