<script lang="ts">
  import Pause from '@lucide/svelte/icons/pause'
  import { api, type Download, type Package } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { accounts } from '$lib/stores/catalog.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { clock, ui } from '$lib/stores/ui.svelte'
  import { displayName, formatBytes, formatDuration, formatFinish, formatPercent, formatSpeed } from '$lib/utils/format'
  import { packageProgress } from '$lib/utils/packages'
  import { isActiveDownload, isFinishedDownload } from '$lib/utils/status'
  import IconButton from '../ui/IconButton.svelte'
  import ProgressBar from '../ui/ProgressBar.svelte'

  /** A package that is moving: downloading here, at the debrid service, or being verified or extracted. */
  let { pkg }: { pkg: Package } = $props()

  const stage = $derived(
    pkg.status === 'debrid'
      ? 'debrid'
      : pkg.status === 'verifying' || pkg.status === 'extracting'
        ? pkg.status
        : 'local',
  )
  const progress = $derived(packageProgress(pkg))
  const active = $derived(pkg.files.filter((f) => isActiveDownload(f.status)))
  const waiting = $derived(pkg.files.filter((f) => f.status === 'queued').length)
  const finished = $derived(pkg.files.filter((f) => isFinishedDownload(f.status)).length)
  const failed = $derived(pkg.files.filter((f) => f.status === 'failed').length)
  /** A package of one file needs no file list: the package bar is the file's. */
  const single = $derived(stage === 'local' && pkg.files.length === 1 ? pkg.files[0] : null)
  const resolving = $derived(single?.status === 'resolving')

  const ratio = $derived(
    stage === 'debrid'
      ? (pkg.debrid?.progress ?? null)
      : stage === 'verifying'
        ? pkg.verify_progress
        : stage === 'extracting'
          ? pkg.extract_progress
          : resolving
            ? null
            : progress.ratio,
  )

  const host = $derived.by(() => {
    const hosts = new Set(pkg.files.map((f) => f.host).filter((h): h is string => !!h))
    return hosts.size === 1 ? [...hosts][0] : null
  })
  const via = $derived.by(() => {
    const provider = (active[0] ?? pkg.files[0])?.via
    return provider && provider !== 'direct' ? accounts.providerName(provider) : null
  })

  const debridLabel = $derived.by(() => {
    const status = pkg.debrid?.status.toLowerCase() ?? ''
    if (status.includes('queue') || status.includes('wait')) return t('dashboard.card.debridQueued')
    if (status.includes('download')) return t('dashboard.card.debridDownloading')
    return t('dashboard.card.debridWorking')
  })

  const finish = $derived(formatFinish(progress.eta, clock.now))
  function fileRatio(file: Download): number | null {
    return file.status === 'resolving' || !file.size ? null : file.downloaded / file.size
  }
</script>

<li class="rounded-panel border border-line bg-panel px-4 py-4 sm:px-5">
  <div class="flex items-start gap-3 sm:gap-4">
    <div class="min-w-0 flex-1">
      <button
        type="button"
        onclick={() => ui.revealPackage(pkg.id)}
        title={t('dashboard.card.open', { name: pkg.name })}
        class="block max-w-full truncate text-left text-lead font-semibold underline-offset-2 hover:underline"
      >
        {pkg.name}
      </button>
      <p class="mt-0.5 flex flex-wrap gap-x-3 text-small text-fg-muted">
        {#if stage === 'debrid'}
          <span title={pkg.debrid?.status}>{debridLabel}</span>
          {#if pkg.debrid?.seeders != null}
            <span>{t('dashboard.seeders', { count: pkg.debrid.seeders })}</span>
          {/if}
        {:else if stage !== 'local'}
          <span>{t(`dashboard.card.${stage}`)}</span>
        {:else if resolving}
          <span class="text-info">{t('status.download.resolving')}</span>
        {:else}
          {#if pkg.files.length > 1}<span>{t('dashboard.card.files', { count: pkg.files.length })}</span>{/if}
          {#if host}<span>{host}</span>{/if}
          {#if via}<span>{t('queue.via', { provider: via })}</span>{/if}
        {/if}
      </p>
    </div>
    <span class="pt-0.5 readout text-[1.625rem] leading-none sm:text-[1.875rem]">{formatPercent(ratio)}</span>
    {#if stage === 'local' || stage === 'debrid'}
      <IconButton
        icon={Pause}
        label={t('dashboard.card.pause', { name: pkg.name })}
        size="sm"
        class="-mr-1.5"
        onclick={() => queue.run(() => api.packages.pause(pkg.id))}
      />
    {/if}
  </div>

  <div class="mt-3">
    <ProgressBar value={ratio} tone={stage === 'local' ? 'signal' : 'info'} size="lg" label={pkg.name} />
  </div>

  {#if stage === 'local' || stage === 'debrid'}
    <div class="mt-2 flex flex-wrap justify-between gap-x-6 gap-y-1 num text-small text-fg-muted">
      <span>
        {#if stage === 'debrid'}
          {pkg.debrid?.size ? formatBytes(pkg.debrid.size) : ''}
        {:else if progress.total > 0}
          {t('dashboard.card.sizeOf', { done: formatBytes(progress.downloaded), total: formatBytes(progress.total) })}
        {:else}
          {formatBytes(progress.downloaded)}
        {/if}
      </span>
      <span class="flex gap-x-4">
        {#if stage === 'debrid'}
          {#if pkg.debrid?.speed}
            <span>{t('dashboard.card.remoteSpeed', { speed: formatSpeed(pkg.debrid.speed) })}</span>
          {/if}
        {:else if progress.speed > 0}
          <span class="font-medium text-fg">{formatSpeed(progress.speed)}</span>
          {#if progress.eta !== null}
            <span title={finish ?? undefined}
              >{t('dashboard.card.left', { duration: formatDuration(progress.eta) })}</span
            >
          {/if}
        {/if}
      </span>
    </div>
  {/if}

  {#if stage === 'local' && !single}
    {#if active.length}
      <ul class="mt-4 divide-y divide-line border-t border-line">
        {#each active as file (file.id)}
          <li class="flex flex-wrap items-center gap-x-4 gap-y-1.5 py-2 text-small">
            <span class="min-w-0 basis-full truncate sm:flex-1 sm:basis-auto" title={displayName(file)}>
              {displayName(file)}
            </span>
            <span class="min-w-20 flex-1 sm:w-40 sm:flex-none">
              <ProgressBar value={fileRatio(file)} size="md" label={displayName(file)} />
            </span>
            {#if file.status === 'resolving'}
              <span class="w-44 text-right text-info sm:w-[13rem]">{t('status.download.resolving')}</span>
            {:else}
              <span class="w-12 text-right num text-fg-muted">{formatPercent(fileRatio(file))}</span>
              <span class="w-20 text-right num">{formatSpeed(file.speed)}</span>
              <span class="hidden w-20 text-right num text-fg-muted sm:block">{formatDuration(file.eta)}</span>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if waiting || finished || failed}
      <p class="flex flex-wrap gap-x-3 text-small text-fg-muted {active.length ? 'border-t border-line pt-2' : 'mt-3'}">
        {#if waiting}<span>{t('dashboard.card.waiting', { count: waiting })}</span>{/if}
        {#if finished}<span>{t('dashboard.card.finished', { count: finished })}</span>{/if}
        {#if failed}<span class="text-bad">{t('dashboard.card.failed', { count: failed })}</span>{/if}
      </p>
    {/if}
  {/if}
</li>
