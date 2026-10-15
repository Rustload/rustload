<script lang="ts">
  import X from '@lucide/svelte/icons/x'
  import type { Snippet } from 'svelte'
  import { t } from '$lib/i18n/index.svelte'
  import IconButton from './IconButton.svelte'

  type Props = {
    open: boolean
    title: string
    description?: string
    size?: 'sm' | 'md' | 'lg' | 'xl'
    /** The body fills a fixed-height dialog and manages its own scrolling (multi-pane layouts). */
    fill?: boolean
    /** Called after the dialog closed, whatever the reason (Escape, backdrop, close button). */
    onclose?: () => void
    /** CSS selector of the element to focus when opening (defaults to the first focusable one). */
    initialFocus?: string
    children: Snippet
    footer?: Snippet
  }

  let {
    open = $bindable(),
    title,
    description,
    size = 'md',
    fill = false,
    onclose,
    initialFocus,
    children,
    footer,
  }: Props = $props()

  let dialog: HTMLDialogElement | undefined = $state()
  const id = $props.id()

  const widths = { sm: 'max-w-md', md: 'max-w-xl', lg: 'max-w-3xl', xl: 'max-w-6xl' }
  const height = $derived(fill ? 'h-[min(92dvh,56rem)]' : 'max-h-[min(90dvh,52rem)]')

  $effect(() => {
    if (!dialog) return
    if (open && !dialog.open) {
      dialog.showModal()
      if (initialFocus) dialog.querySelector<HTMLElement>(initialFocus)?.focus()
    } else if (!open && dialog.open) {
      dialog.close()
    }
  })

  function handleClose() {
    if (!open) return
    open = false
    onclose?.()
  }

  // Close on backdrop clicks only (the dialog element itself is the backdrop area).
  function onpointerdown(event: PointerEvent) {
    if (event.target === dialog) dialog?.close()
  }
</script>

<dialog
  bind:this={dialog}
  onclose={handleClose}
  {onpointerdown}
  aria-labelledby="{id}-title"
  aria-describedby={description ? `${id}-description` : undefined}
  class="dialog m-auto w-[calc(100%-1.5rem)] {widths[size]} {height} overflow-hidden rounded-panel
    border border-line bg-panel p-0 text-fg shadow-pop backdrop:bg-overlay"
>
  {#if open}
    <div class="flex {height} flex-col">
      <header class="flex items-start justify-between gap-4 px-5 pt-4 pb-3">
        <div class="min-w-0">
          <h2 id="{id}-title" class="text-lg font-semibold heading">{title}</h2>
          {#if description}
            <p id="{id}-description" class="mt-1 text-fg-muted">{description}</p>
          {/if}
        </div>
        <IconButton icon={X} label={t('common.close')} size="sm" class="-mr-1.5" onclick={() => dialog?.close()} />
      </header>
      <div class="min-h-0 flex-1 {fill ? 'overflow-y-auto lg:overflow-hidden' : 'overflow-y-auto px-5 pb-5'}">
        {@render children()}
      </div>
      {#if footer}
        <footer class="flex flex-wrap items-center justify-end gap-2 border-t border-line bg-bg/40 px-5 py-3">
          {@render footer()}
        </footer>
      {/if}
    </div>
  {/if}
</dialog>

<style>
  .dialog[open] {
    animation: dialog-in 140ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .dialog[open]::backdrop {
    animation: backdrop-in 140ms ease-out;
  }
  @keyframes dialog-in {
    from {
      opacity: 0;
      transform: translateY(6px) scale(0.985);
    }
  }
  @keyframes backdrop-in {
    from {
      opacity: 0;
    }
  }
</style>
