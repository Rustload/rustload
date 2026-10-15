<script lang="ts" module>
  import type { LucideIcon } from '@lucide/svelte'

  export type MenuItem =
    { label: string; icon?: LucideIcon; onselect: () => void; danger?: boolean; disabled?: boolean } | 'separator'
</script>

<script lang="ts">
  import Ellipsis from '@lucide/svelte/icons/ellipsis'
  import { tick } from 'svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { iconButtonClass, type ButtonSize } from './styles'

  type Props = {
    items: MenuItem[]
    label?: string
    icon?: LucideIcon
    size?: ButtonSize
    align?: 'start' | 'end'
  }

  let { items, label, icon: Icon = Ellipsis, size = 'sm', align = 'end' }: Props = $props()

  const name = $derived(label ?? t('common.more'))

  const id = $props.id()
  let trigger: HTMLButtonElement | undefined = $state()
  let popover: HTMLDivElement | undefined = $state()
  let open = $state(false)

  // Uses the Popover API: top layer, light dismiss and Escape handling come for free.
  function place() {
    if (!trigger || !popover) return
    const rect = trigger.getBoundingClientRect()
    const width = popover.offsetWidth
    const height = popover.offsetHeight
    let left = align === 'end' ? rect.right - width : rect.left
    left = Math.max(8, Math.min(left, window.innerWidth - width - 8))
    let top = rect.bottom + 4
    if (top + height > window.innerHeight - 8) top = Math.max(8, rect.top - height - 4)
    popover.style.left = `${left}px`
    popover.style.top = `${top}px`
  }

  // Items are only rendered while open, keeping long lists of rows light.
  async function ontoggle(event: ToggleEvent) {
    open = event.newState === 'open'
    if (open) {
      await tick()
      place()
      popover?.querySelector<HTMLElement>('[role="menuitem"]:not(:disabled)')?.focus()
    }
  }

  function select(item: Exclude<MenuItem, 'separator'>) {
    popover?.hidePopover()
    trigger?.focus()
    item.onselect()
  }

  function onkeydown(event: KeyboardEvent) {
    const entries = [...(popover?.querySelectorAll<HTMLElement>('[role="menuitem"]:not(:disabled)') ?? [])]
    const index = entries.indexOf(document.activeElement as HTMLElement)
    const go = (i: number) => entries[(i + entries.length) % entries.length]?.focus()
    if (event.key === 'ArrowDown') go(index + 1)
    else if (event.key === 'ArrowUp') go(index - 1)
    else if (event.key === 'Home') go(0)
    else if (event.key === 'End') go(entries.length - 1)
    else if (event.key === 'Tab') popover?.hidePopover()
    else return
    event.preventDefault()
  }
</script>

<button
  bind:this={trigger}
  type="button"
  popovertarget="{id}-menu"
  aria-haspopup="menu"
  aria-expanded={open}
  aria-label={name}
  title={name}
  class={iconButtonClass('ghost', size, open ? 'bg-sunken text-fg' : '')}
>
  <Icon size={size === 'sm' ? 15 : 17} aria-hidden="true" />
</button>

<div
  bind:this={popover}
  id="{id}-menu"
  popover="auto"
  role="menu"
  tabindex="-1"
  {ontoggle}
  {onkeydown}
  class="fixed m-0 min-w-48 rounded-panel border border-line bg-raised p-1 text-fg shadow-pop"
>
  {#if open}
    {#each items as item, i (i)}
      {#if item === 'separator'}
        <div class="my-1 h-px bg-line" role="separator"></div>
      {:else}
        <button
          type="button"
          role="menuitem"
          disabled={item.disabled}
          onclick={() => select(item)}
          class="flex w-full items-center gap-2.5 rounded-chip px-2.5 py-1.5 text-left text-sm
            focus-visible:outline-none disabled:opacity-40
            {item.danger ? 'text-bad hover:bg-bad-wash focus:bg-bad-wash' : 'hover:bg-sunken focus:bg-sunken'}"
        >
          {#if item.icon}
            <item.icon size={15} class={item.danger ? '' : 'text-fg-muted'} aria-hidden="true" />
          {/if}
          {item.label}
        </button>
      {/if}
    {/each}
  {/if}
</div>
