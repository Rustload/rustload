<script lang="ts">
  import FileArchive from '@lucide/svelte/icons/file-archive'
  import RotateCw from '@lucide/svelte/icons/rotate-cw'
  import ShieldAlert from '@lucide/svelte/icons/shield-alert'
  import ShieldCheck from '@lucide/svelte/icons/shield-check'
  import Trash2 from '@lucide/svelte/icons/trash-2'
  import { removeHistoryEntry } from '$lib/actions'
  import { api, type Package } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { categories } from '$lib/stores/catalog.svelte'
  import { history } from '$lib/stores/history.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { toasts } from '$lib/stores/toasts.svelte'
  import {
    displayName,
    formatBytes,
    formatDateTime,
    formatDuration,
    formatRelative,
    formatSpeed,
  } from '$lib/utils/format'
  import { packageDuration } from '$lib/utils/packages'
  import { downloadTone, toneFill } from '$lib/utils/status'
  import DisclosureButton from '../ui/DisclosureButton.svelte'
  import Menu, { type MenuItem } from '../ui/Menu.svelte'
  import StatusBadge from '../ui/StatusBadge.svelte'

  let { pkg }: { pkg: Package } = $props()

  let expanded = $state(false)

  const size = $derived(pkg.files.reduce((sum, f) => sum + (f.size ?? f.downloaded), 0) || pkg.total_size)
  const duration = $derived(packageDuration(pkg))
  const avgSpeed = $derived(duration && duration > 0 ? size / duration : null)
  const category = $derived(categories.byId(pkg.category_id))
  const filesId = $derived(`history-${pkg.id}-files`)

  async function redownload() {
    try {
      const { packages } = await api.history.redownload(pkg.id)
      toasts.success(t('add.added', { count: packages.length }))
      queue.invalidate()
    } catch (err) {
      toasts.error(err)
    }
  }

  const items: MenuItem[] = $derived([
    { label: t('history.redownload'), icon: RotateCw, onselect: redownload },
    ...(pkg.extract
      ? [
          {
            label: t('queue.extractAgain'),
            icon: FileArchive,
            onselect: () => history.run(() => api.packages.extract(pkg.id)),
          },
        ]
      : []),
    'separator' as const,
    { label: t('history.removeEntry'), icon: Trash2, danger: true, onselect: () => removeHistoryEntry(pkg) },
  ])
</script>

<li class="border-b border-line last:border-b-0">
  <div class="flex items-start gap-2 py-3 pr-1 sm:items-center">
    <DisclosureButton bind:expanded controls={filesId} />

    <div class="min-w-0 flex-1">
      <div class="flex min-w-0 items-center gap-2.5">
        <span class="truncate font-medium">{pkg.name}</span>
        {#if pkg.status !== 'completed' || pkg.error}
          <StatusBadge kind="package" status={pkg.status === 'completed' ? 'failed' : pkg.status} />
        {/if}
      </div>
      <p class="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5 num text-small text-fg-muted">
        <span>{category?.name ?? t('queue.automatic')}</span>
        <span>{formatBytes(size)}</span>
        <span>{t('queue.files', { count: pkg.files.length })}</span>
        {#if duration !== null}<span class="hidden md:inline"
            >{t('history.duration', { duration: formatDuration(duration) })}</span
          >{/if}
        {#if avgSpeed}<span class="hidden md:inline">{t('history.avgSpeed', { speed: formatSpeed(avgSpeed) })}</span
          >{/if}
      </p>
      {#if pkg.error}
        <p class="mt-1 text-small break-words text-bad">{pkg.error}</p>
      {/if}
    </div>

    <time
      class="hidden shrink-0 text-right num text-small text-fg-muted sm:block"
      datetime={pkg.finished_at ? new Date(pkg.finished_at * 1000).toISOString() : undefined}
      title={formatDateTime(pkg.finished_at)}
    >
      {formatRelative(pkg.finished_at)}
    </time>
    <Menu {items} />
  </div>

  {#if expanded}
    <ul id={filesId} class="mb-3 ml-9 space-y-2 rounded-control bg-sunken px-3 py-2.5">
      {#if pkg.dest_dir}
        <li class="font-mono text-xs break-all text-fg-muted">{pkg.dest_dir}</li>
      {/if}
      {#each pkg.files as file (file.id)}
        <li class="grid grid-cols-[0.5rem_minmax(0,1fr)_auto] items-start gap-x-3">
          <span class="mt-1.5 size-1.5 rounded-full {toneFill[downloadTone[file.status]]}" aria-hidden="true"></span>
          <div class="min-w-0">
            <p class="flex min-w-0 items-center gap-1.5 text-small">
              <span class="truncate">{displayName(file)}</span>
              {#if file.checksum?.status === 'ok'}
                <ShieldCheck
                  size={13}
                  class="shrink-0 text-ok"
                  aria-label={t('history.checksumOk', { algo: file.checksum.algo.toUpperCase() })}
                />
              {:else if file.checksum}
                <ShieldAlert size={13} class="shrink-0 text-bad" aria-label={t('queue.checksumBad')} />
              {/if}
            </p>
            {#if file.path}
              <p class="truncate font-mono text-xs text-fg-faint" title={file.path}>{file.path}</p>
            {/if}
            {#if file.error}<p class="text-xs text-bad">{file.error}</p>{/if}
          </div>
          <span class="num text-xs text-fg-muted">{formatBytes(file.size)}</span>
        </li>
      {/each}
    </ul>
  {/if}
</li>
