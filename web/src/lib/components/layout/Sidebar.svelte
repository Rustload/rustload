<script lang="ts">
  import LogOut from '@lucide/svelte/icons/log-out'
  import Moon from '@lucide/svelte/icons/moon'
  import Search from '@lucide/svelte/icons/search'
  import Sun from '@lucide/svelte/icons/sun'
  import { t } from '$lib/i18n/index.svelte'
  import { router } from '$lib/router.svelte'
  import { auth } from '$lib/stores/auth.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { system } from '$lib/stores/system.svelte'
  import { theme } from '$lib/stores/theme.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { isApple } from '$lib/utils/platform'
  import IconButton from '../ui/IconButton.svelte'
  import Logo from './Logo.svelte'
  import { NAV_ITEMS } from './nav'

  const failed = $derived(queue.packages.filter((p) => p.status === 'failed').length)
</script>

<aside class="sticky top-0 hidden h-dvh flex-col border-r border-line bg-panel lg:flex">
  <a href={router.href('/')} class="flex h-14 items-center px-5" aria-label="Rustload">
    <Logo size={20} withName />
  </a>

  <div class="px-3 pt-2">
    <button
      type="button"
      onclick={() => (ui.paletteOpen = true)}
      class="flex h-9 w-full items-center gap-2.5 rounded-control border border-line bg-bg px-3 text-small
        text-fg-faint transition-colors hover:border-line-strong hover:text-fg-muted"
    >
      <Search size={15} aria-hidden="true" />
      <span class="min-w-0 flex-1 truncate text-left">{t('palette.trigger')}</span>
      <kbd class="num text-micro">{isApple ? '⌘K' : 'Ctrl K'}</kbd>
    </button>
  </div>

  <nav aria-label={t('nav.label')} class="flex-1 px-3 pt-3">
    <ul class="space-y-0.5">
      {#each NAV_ITEMS as item (item.path)}
        {@const active = router.isActive(item.path)}
        <li>
          <a
            href={router.href(item.path)}
            aria-current={active ? 'page' : undefined}
            class="group relative flex h-9 items-center gap-3 rounded-control px-3 font-medium transition-colors
              {active ? 'bg-sunken text-fg' : 'text-fg-muted hover:bg-sunken/60 hover:text-fg'}"
          >
            {#if active}
              <span class="absolute inset-y-2 -left-3 w-0.5 rounded-r bg-fg" aria-hidden="true"></span>
            {/if}
            <item.icon size={17} strokeWidth={active ? 2.25 : 2} aria-hidden="true" />
            <span class="flex-1">{t(item.label)}</span>
            {#if item.path === '/queue' && queue.packages.length > 0}
              {#if failed}
                <span class="size-1.5 rounded-full bg-bad" title={t('status.package.failed')} aria-hidden="true"></span>
              {/if}
              <span class="num text-small text-fg-faint">{queue.packages.length}</span>
            {/if}
          </a>
        </li>
      {/each}
    </ul>
  </nav>

  <div class="space-y-1 border-t border-line px-3 py-3">
    <div class="flex items-center gap-1">
      <IconButton
        icon={theme.dark ? Sun : Moon}
        label={theme.dark ? t('shell.themeLight') : t('shell.themeDark')}
        size="sm"
        onclick={() => settings.setTheme(theme.dark ? 'light' : 'dark')}
      />
      {#if auth.enabled && auth.username}
        <span
          class="min-w-0 flex-1 truncate px-1 text-small text-fg-muted"
          title={t('shell.signedInAs', { user: auth.username })}
        >
          {auth.username}
        </span>
        {#if !auth.desktop}
          <IconButton icon={LogOut} label={t('shell.signOut')} size="sm" onclick={() => auth.logout()} />
        {/if}
      {:else}
        <span class="flex-1"></span>
      {/if}
    </div>
    {#if system.info}
      <p class="px-1 num text-xs text-fg-faint">Rustload {system.info.version}</p>
    {/if}
  </div>
</aside>
