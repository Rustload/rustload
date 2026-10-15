<script lang="ts">
  import CircleAlert from '@lucide/svelte/icons/circle-alert'
  import HardDrive from '@lucide/svelte/icons/hard-drive'
  import KeyRound from '@lucide/svelte/icons/key-round'
  import { api, type Package } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { router } from '$lib/router.svelte'
  import { accounts } from '$lib/stores/catalog.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { system } from '$lib/stores/system.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import Button from '../ui/Button.svelte'
  import SectionHeading from './SectionHeading.svelte'

  /** What stopped and waits for the user: failed downloads, a full disk, an account in error. */

  const failing = $derived(
    queue.packages
      .map((pkg) => ({ pkg, failed: pkg.files.filter((f) => f.status === 'failed') }))
      .filter(({ pkg, failed }) => pkg.status === 'failed' || failed.length > 0),
  )
  const brokenAccounts = $derived(accounts.list.filter((a) => a.enabled && a.last_error))
  const count = $derived(failing.length + brokenAccounts.length + (system.diskLow ? 1 : 0))

  function reason(pkg: Package, failed: number): string {
    if (pkg.status !== 'failed') return t('dashboard.attention.someFailed', { count: failed })
    return pkg.error ?? pkg.files.find((f) => f.error)?.error ?? t('dashboard.attention.failed')
  }
</script>

{#snippet item(icon: typeof CircleAlert, title: string, body: string, onopen: () => void)}
  {@const Icon = icon}
  <div class="flex min-w-0 flex-1 basis-64 gap-3">
    <Icon size={17} class="mt-0.5 shrink-0 text-bad" aria-hidden="true" />
    <div class="min-w-0">
      <button
        type="button"
        onclick={onopen}
        class="block max-w-full truncate text-left font-medium underline-offset-2 hover:underline"
      >
        {title}
      </button>
      <p class="text-small break-words text-fg-muted">{body}</p>
    </div>
  </div>
{/snippet}

{#if count}
  <section aria-labelledby="attention">
    <SectionHeading id="attention" title={t('dashboard.attention.title')} {count} />
    <ul class="divide-y divide-line rounded-panel border border-bad/40 bg-panel">
      {#if system.diskLow}
        <li class="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 sm:px-5">
          {@render item(HardDrive, t('dashboard.attention.diskLow'), t('dashboard.diskLow'), () =>
            router.go('/settings/folders'),
          )}
        </li>
      {/if}
      {#each failing as { pkg, failed } (pkg.id)}
        <li class="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 sm:px-5">
          {@render item(CircleAlert, pkg.name, reason(pkg, failed.length), () => ui.revealPackage(pkg.id))}
          <Button size="sm" variant="secondary" onclick={() => queue.run(() => api.packages.retry(pkg.id))}>
            {t('common.retry')}
          </Button>
        </li>
      {/each}
      {#each brokenAccounts as account (account.id)}
        <li class="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 sm:px-5">
          {@render item(
            KeyRound,
            t('dashboard.attention.account', { name: account.label }),
            account.last_error ?? '',
            () => router.go('/settings/accounts'),
          )}
        </li>
      {/each}
    </ul>
  </section>
{/if}
