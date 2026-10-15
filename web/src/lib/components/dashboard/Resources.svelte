<script lang="ts">
  import HardDrive from '@lucide/svelte/icons/hard-drive'
  import { t } from '$lib/i18n/index.svelte'
  import { router } from '$lib/router.svelte'
  import { accounts } from '$lib/stores/catalog.svelte'
  import { system } from '$lib/stores/system.svelte'
  import { clock } from '$lib/stores/ui.svelte'
  import { formatBytes } from '$lib/utils/format'
  import ProviderMark from '../settings/ProviderMark.svelte'
  import ProgressBar from '../ui/ProgressBar.svelte'
  import SectionHeading from './SectionHeading.svelte'

  /** Free disk space and the state of each account: what decides how much more can come down. */

  const enabled = $derived(accounts.list.filter((a) => a.enabled))
  const disk = $derived.by(() => {
    const free = system.stats?.disk_free ?? system.info?.disk?.free ?? null
    const total = system.stats?.disk_total ?? system.info?.disk?.total ?? null
    return free !== null && total ? { free, total, used: 1 - free / total } : null
  })

  function daysLeft(until: number | null): number | null {
    return until === null ? null : Math.max(0, Math.floor((until - clock.now / 1000) / 86400))
  }
</script>

{#if disk || enabled.length}
  <section aria-labelledby="resources">
    <SectionHeading id="resources" title={enabled.length ? t('dashboard.resources') : t('dashboard.disk')}>
      {#snippet aside()}
        {#if enabled.length}
          <a href={router.href('/settings/accounts')} class="text-small text-fg-muted hover:text-fg">
            {t('dashboard.manageAccounts')}
          </a>
        {/if}
      {/snippet}
    </SectionHeading>
    <ul class="grid gap-x-8 gap-y-1 border-y border-line py-2 sm:grid-cols-2 xl:grid-cols-3">
      {#if disk}
        <li class="flex items-center gap-3 py-1.5">
          <span class="grid size-7 shrink-0 place-items-center rounded-control bg-sunken text-fg-muted">
            <HardDrive size={15} aria-hidden="true" />
          </span>
          <div class="min-w-0 flex-1">
            <p class="flex items-baseline gap-2">
              <span class="num text-small font-medium">{t('dashboard.diskFree', { size: formatBytes(disk.free) })}</span
              >
              <span class="num text-xs text-fg-faint">{t('dashboard.diskOf', { total: formatBytes(disk.total) })}</span>
            </p>
            <ProgressBar
              value={disk.used}
              tone={disk.used > 0.92 ? 'bad' : 'muted'}
              size="xs"
              class="mt-1.5 max-w-48"
              label={t('dashboard.disk')}
            />
          </div>
        </li>
      {/if}
      {#each enabled as account (account.id)}
        {@const days = daysLeft(account.premium_until)}
        <li class="flex items-center gap-3 py-1.5">
          <ProviderMark name={accounts.providerName(account.provider)} size="sm" />
          <div class="min-w-0">
            <p class="truncate text-small font-medium">{account.label}</p>
            <p class="flex flex-wrap gap-x-3 num text-xs text-fg-muted">
              {#if account.last_error}
                <span class="truncate text-bad">{account.last_error}</span>
              {:else if !account.premium}
                <span class="text-warn">{t('settings.accounts.free')}</span>
              {:else}
                {#if days !== null}
                  <span class={days < 7 ? 'text-warn' : ''}>{t('dashboard.daysLeft', { count: days })}</span>
                {/if}
                {#if account.traffic_left !== null}
                  <span>{t('settings.accounts.trafficLeft', { size: formatBytes(account.traffic_left) })}</span>
                {/if}
              {/if}
            </p>
          </div>
        </li>
      {/each}
    </ul>
  </section>
{/if}
