<script lang="ts">
  import CircleCheck from '@lucide/svelte/icons/circle-check'
  import CircleX from '@lucide/svelte/icons/circle-x'
  import ShieldCheck from '@lucide/svelte/icons/shield-check'
  import { api, type Package } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { router } from '$lib/router.svelte'
  import { history } from '$lib/stores/history.svelte'
  import { clock } from '$lib/stores/ui.svelte'
  import { formatBytes, formatRelative } from '$lib/utils/format'
  import SectionHeading from './SectionHeading.svelte'

  /** `hideEmpty`: render nothing until something has finished (a fresh install). */
  let { items = $bindable([]), hideEmpty = false }: { items?: Package[]; hideEmpty?: boolean } = $props()

  // Refetch whenever the history changes (a package finished, an entry was removed…).
  let request = 0
  $effect(() => {
    void history.version
    const id = ++request
    api.history
      .list('', 5, 0)
      .then((page) => {
        if (id === request) items = page.items
      })
      .catch(() => {})
  })

  function verified(pkg: Package): boolean {
    return pkg.files.length > 0 && pkg.files.every((f) => f.checksum?.status === 'ok')
  }

  function open(pkg: Package) {
    history.search(pkg.name)
    router.go('/history')
  }
</script>

<section aria-labelledby="recent" class:hidden={hideEmpty && !items.length}>
  <SectionHeading id="recent" title={t('dashboard.recent')} />
  {#if items.length}
    <ul class="divide-y divide-line border-y border-line">
      {#each items as pkg (pkg.id)}
        {@const failed = pkg.status === 'failed'}
        <li>
          <button
            type="button"
            onclick={() => open(pkg)}
            class="flex w-full items-center gap-3 py-2 text-left hover:bg-sunken/50"
          >
            {#if failed}
              <CircleX size={15} class="shrink-0 text-bad" aria-label={t('status.package.failed')} />
            {:else}
              <CircleCheck size={15} class="shrink-0 text-ok" aria-label={t('status.package.completed')} />
            {/if}
            <span class="min-w-0 flex-1">
              <span class="block truncate text-small">{pkg.name}</span>
              <span class="flex gap-3 text-xs text-fg-faint">
                <span>{formatRelative(pkg.finished_at, clock.now)}</span>
                {#if failed && pkg.error}
                  <span class="truncate text-bad">{pkg.error}</span>
                {:else if verified(pkg)}
                  <span class="flex items-center gap-1 text-ok">
                    <ShieldCheck size={12} aria-hidden="true" />{t('dashboard.verified')}
                  </span>
                {/if}
              </span>
            </span>
            <span class="shrink-0 num text-xs text-fg-muted">{formatBytes(pkg.total_size)}</span>
          </button>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="border-y border-line py-3 text-small text-fg-faint">{t('dashboard.nothingRecent')}</p>
  {/if}
</section>
