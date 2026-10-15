<script lang="ts">
  import { t } from '$lib/i18n/index.svelte'
  import { router } from '$lib/router.svelte'
  import { NAV_ITEMS } from './nav'
</script>

<nav
  aria-label={t('nav.label')}
  class="fixed inset-x-0 bottom-0 z-30 border-t border-line bg-panel pb-[env(safe-area-inset-bottom)] lg:hidden"
>
  <ul class="grid grid-cols-5">
    {#each NAV_ITEMS.filter((i) => i.mobile) as item (item.path)}
      {@const active = router.isActive(item.path)}
      <li>
        <a
          href={router.href(item.path)}
          aria-current={active ? 'page' : undefined}
          class="flex h-14 flex-col items-center justify-center gap-1 text-micro font-medium
            {active ? 'text-fg' : 'text-fg-faint'}"
        >
          <item.icon size={19} strokeWidth={active ? 2.25 : 2} aria-hidden="true" />
          <span class="max-w-full truncate px-1">{t(item.short ?? item.label)}</span>
        </a>
      </li>
    {/each}
  </ul>
</nav>
