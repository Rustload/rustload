<script lang="ts">
  import ArrowDown from '@lucide/svelte/icons/arrow-down'
  import ArrowDownToLine from '@lucide/svelte/icons/arrow-down-to-line'
  import ArrowUp from '@lucide/svelte/icons/arrow-up'
  import ArrowUpToLine from '@lucide/svelte/icons/arrow-up-to-line'
  import FileArchive from '@lucide/svelte/icons/file-archive'
  import Folder from '@lucide/svelte/icons/folder'
  import GripVertical from '@lucide/svelte/icons/grip-vertical'
  import Magnet from '@lucide/svelte/icons/magnet'
  import Pause from '@lucide/svelte/icons/pause'
  import Pencil from '@lucide/svelte/icons/pencil'
  import Play from '@lucide/svelte/icons/play'
  import RotateCcw from '@lucide/svelte/icons/rotate-ccw'
  import Trash2 from '@lucide/svelte/icons/trash-2'
  import { deletePackages } from '$lib/actions'
  import { api, type Package } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { categories } from '$lib/stores/catalog.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { formatBytes, formatDuration, formatPercent, formatSpeed } from '$lib/utils/format'
  import { isPausedPackage, packageProgress } from '$lib/utils/packages'
  import { isFinishedDownload, packageTone } from '$lib/utils/status'
  import DisclosureButton from '../ui/DisclosureButton.svelte'
  import IconButton from '../ui/IconButton.svelte'
  import Menu, { type MenuItem } from '../ui/Menu.svelte'
  import ProgressBar from '../ui/ProgressBar.svelte'
  import StatusBadge from '../ui/StatusBadge.svelte'
  import FileRow from './FileRow.svelte'

  type Props = {
    pkg: Package
    first: boolean
    last: boolean
    selected: boolean
    onselect: (selected: boolean) => void
    onedit: () => void
    /** Starts a drag from the handle; the queue page owns the drop logic. Null disables reordering. */
    ondragstart: ((event: DragEvent) => void) | null
    expanded?: boolean
    /** Briefly outlines the row (it was opened from the command palette). */
    highlight?: boolean
  }

  let {
    pkg,
    first,
    last,
    selected,
    onselect,
    onedit,
    ondragstart,
    expanded = $bindable(false),
    highlight = false,
  }: Props = $props()

  const progress = $derived(packageProgress(pkg))
  const paused = $derived(isPausedPackage(pkg))
  const hasFailed = $derived(pkg.files.some((f) => f.status === 'failed'))
  const allDone = $derived(pkg.files.length > 0 && pkg.files.every((f) => isFinishedDownload(f.status)))
  const category = $derived(categories.byId(pkg.category_id))
  const destination = $derived(pkg.dest_dir ?? category?.resolved_path ?? null)
  const barTone = $derived(pkg.status === 'downloading' ? 'signal' : packageTone[pkg.status])
  // Indeterminate only while something is actually happening without a known size.
  const barValue = $derived(
    pkg.status === 'debrid'
      ? (pkg.debrid?.progress ?? null)
      : (progress.ratio ?? (pkg.status === 'downloading' ? null : 0)),
  )
  const filesId = $derived(`package-${pkg.id}-files`)

  const items: MenuItem[] = $derived([
    { label: t('common.edit'), icon: Pencil, onselect: onedit },
    ...(hasFailed
      ? [{ label: t('common.retry'), icon: RotateCcw, onselect: () => queue.run(() => api.packages.retry(pkg.id)) }]
      : []),
    ...(pkg.extract && allDone
      ? [
          {
            label: t('queue.extractAgain'),
            icon: FileArchive,
            onselect: () => queue.run(() => api.packages.extract(pkg.id)),
          },
        ]
      : []),
    'separator' as const,
    { label: t('queue.moveTop'), icon: ArrowUpToLine, disabled: first, onselect: () => queue.move(pkg.id, 'top') },
    { label: t('queue.moveUp'), icon: ArrowUp, disabled: first, onselect: () => queue.move(pkg.id, 'up') },
    { label: t('queue.moveDown'), icon: ArrowDown, disabled: last, onselect: () => queue.move(pkg.id, 'down') },
    {
      label: t('queue.moveBottom'),
      icon: ArrowDownToLine,
      disabled: last,
      onselect: () => queue.move(pkg.id, 'bottom'),
    },
    'separator' as const,
    { label: t('common.delete'), icon: Trash2, danger: true, onselect: () => deletePackages([pkg]) },
  ])

  function togglePause() {
    queue.run(() => (paused ? api.packages.resume(pkg.id) : api.packages.pause(pkg.id)))
  }
</script>

<div
  class="rounded-control border bg-panel transition-shadow duration-500
    {selected ? 'border-line-strong' : 'border-line'} {highlight ? 'ring-2 ring-info/60' : ''}"
>
  <div class="flex items-start gap-2 py-2.5 pr-2 pl-2 sm:items-center">
    <div class="flex shrink-0 items-center gap-0.5 pt-0.5 sm:pt-0">
      <input
        type="checkbox"
        checked={selected}
        onchange={(e) => onselect(e.currentTarget.checked)}
        aria-label={t('queue.select', { name: pkg.name })}
        class="mx-1.5 size-4 cursor-pointer accent-[var(--fg)]"
      />
      {#if ondragstart}
        <span
          draggable="true"
          {ondragstart}
          role="presentation"
          title={t('queue.dragHandle')}
          class="hidden cursor-grab rounded-chip p-0.5 text-fg-faint hover:text-fg-muted active:cursor-grabbing sm:block"
        >
          <GripVertical size={15} aria-hidden="true" />
        </span>
      {/if}
      <DisclosureButton bind:expanded controls={filesId} />
    </div>

    <div class="min-w-0 flex-1 md:grid md:grid-cols-[minmax(0,1fr)_11rem_8rem] md:items-center md:gap-x-6">
      <div class="min-w-0">
        <div class="flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1 sm:flex-nowrap">
          <button
            type="button"
            class="line-clamp-2 min-w-0 text-left font-medium [overflow-wrap:anywhere] hover:underline sm:truncate"
            onclick={() => (expanded = !expanded)}
          >
            {pkg.name}
          </button>
          <StatusBadge kind="package" status={pkg.status} />
        </div>
        <p class="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-3.5 gap-y-0.5 text-small text-fg-muted">
          <span class="flex min-w-0 items-center gap-1.5" title={destination ?? t('queue.destinationPending')}>
            <Folder size={13} class="shrink-0 text-fg-faint" aria-hidden="true" />
            <span class="truncate">{category?.name ?? t('queue.automatic')}</span>
            {#if pkg.dest_dir}<span class="hidden truncate text-fg-faint xl:inline">{pkg.dest_dir}</span>{/if}
          </span>
          {#if pkg.source_kind !== 'links'}
            <span class="flex items-center gap-1.5">
              <Magnet size={13} class="text-fg-faint" aria-hidden="true" />{t(`source.${pkg.source_kind}`)}
            </span>
          {/if}
          {#if pkg.files.length || pkg.source_kind === 'links'}
            <span class="num">{t('queue.files', { count: pkg.files.length })}</span>
          {/if}
        </p>
        {#if pkg.error}
          <p class="mt-1 text-small break-words text-bad">{pkg.error}</p>
        {/if}
      </div>

      <div class="mt-2 md:mt-0">
        <div class="flex justify-between gap-2 num text-xs text-fg-muted">
          <span>
            {progress.total > 0 ? `${formatBytes(progress.downloaded)} / ${formatBytes(progress.total)}` : '—'}
          </span>
          <span>{formatPercent(progress.ratio)}</span>
        </div>
        <ProgressBar value={barValue} tone={barTone} class="mt-1" label={pkg.name} />
      </div>

      <div class="mt-1 num text-xs text-fg-muted md:mt-0 md:text-right">
        {#if progress.speed > 0}
          <span class="font-medium text-fg">{formatSpeed(progress.speed)}</span>
          <span class="ml-2 md:ml-0 md:block">{formatDuration(progress.eta)}</span>
        {:else if pkg.status === 'debrid' && pkg.debrid}
          <span class="text-info">{formatPercent(pkg.debrid.progress)}</span>
        {/if}
      </div>
    </div>

    <div class="flex shrink-0 items-center">
      <IconButton
        icon={paused ? Play : Pause}
        label={paused ? t('common.resume') : t('common.pause')}
        size="sm"
        onclick={togglePause}
      />
      <Menu {items} />
    </div>
  </div>

  {#if expanded}
    <ul id={filesId} class="divide-y divide-line border-t border-line px-3 sm:pl-[4.75rem]">
      {#each pkg.files as file (file.id)}
        <FileRow {file} />
      {/each}
    </ul>
  {/if}
</div>
