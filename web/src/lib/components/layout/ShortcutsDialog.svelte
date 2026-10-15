<script lang="ts">
  import { t } from '$lib/i18n/index.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { isApple } from '$lib/utils/platform'
  import Dialog from '../ui/Dialog.svelte'
  import { NAV_ITEMS } from './nav'

  const general = $derived([
    { keys: [isApple ? '⌘' : 'Ctrl', 'K'], label: t('shortcuts.palette') },
    { keys: ['/'], label: t('shortcuts.palette') },
    { keys: ['N'], label: t('shortcuts.add') },
    { keys: [isApple ? '⌘' : 'Ctrl', 'V'], label: t('shortcuts.paste') },
    { keys: ['?'], label: t('shortcuts.help') },
  ])
</script>

{#snippet combo(keys: string[])}
  <span class="flex shrink-0 items-center gap-1">
    {#each keys as key, i (i)}
      <kbd
        class="grid h-6 min-w-6 place-items-center rounded-chip border border-line bg-sunken px-1.5 text-xs text-fg-muted"
      >
        {key}
      </kbd>
    {/each}
  </span>
{/snippet}

<Dialog bind:open={ui.shortcutsOpen} title={t('shortcuts.title')} size="md">
  <div class="grid gap-x-10 gap-y-6 sm:grid-cols-2">
    <section>
      <h3 class="mb-2 text-small text-fg-muted">{t('shortcuts.general')}</h3>
      <ul class="space-y-2">
        {#each general as item, i (i)}
          <li class="flex items-center justify-between gap-4">
            <span>{item.label}</span>
            {@render combo(item.keys)}
          </li>
        {/each}
      </ul>
    </section>
    <section>
      <h3 class="mb-2 text-small text-fg-muted">{t('shortcuts.navigation')}</h3>
      <ul class="space-y-2">
        {#each NAV_ITEMS as item (item.path)}
          <li class="flex items-center justify-between gap-4">
            <span>{t(item.label)}</span>
            {@render combo(item.keys)}
          </li>
        {/each}
      </ul>
    </section>
  </div>
  <p class="mt-6 text-small text-fg-muted">{t('shortcuts.pasteHint')}</p>
</Dialog>
