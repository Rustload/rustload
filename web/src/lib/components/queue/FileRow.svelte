<script lang="ts">
  import Pause from '@lucide/svelte/icons/pause'
  import Play from '@lucide/svelte/icons/play'
  import RotateCcw from '@lucide/svelte/icons/rotate-ccw'
  import ShieldAlert from '@lucide/svelte/icons/shield-alert'
  import ShieldCheck from '@lucide/svelte/icons/shield-check'
  import Trash2 from '@lucide/svelte/icons/trash-2'
  import { deleteFile } from '$lib/actions'
  import { api, type Download } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { accounts } from '$lib/stores/catalog.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { clock } from '$lib/stores/ui.svelte'
  import { displayName, formatBytes, formatDuration, formatPercent, formatSpeed } from '$lib/utils/format'
  import { downloadTone, toneFill, toneText } from '$lib/utils/status'
  import SegmentStrip from '../dashboard/SegmentStrip.svelte'
  import Menu, { type MenuItem } from '../ui/Menu.svelte'
  import ProgressBar from '../ui/ProgressBar.svelte'

  let { file }: { file: Download } = $props()

  const tone = $derived(downloadTone[file.status])
  const ratio = $derived(file.size ? file.downloaded / file.size : null)
  const transferring = $derived(file.status === 'downloading')
  const retryIn = $derived(file.retry_at !== null ? Math.max(0, file.retry_at - clock.now / 1000) : null)
  const via = $derived(
    file.via && file.via !== 'direct' ? t('queue.via', { provider: accounts.providerName(file.via) }) : null,
  )

  const items: MenuItem[] = $derived([
    ...(file.status === 'paused'
      ? [{ label: t('common.resume'), icon: Play, onselect: () => queue.run(() => api.downloads.resume(file.id)) }]
      : []),
    ...(['queued', 'resolving', 'downloading'].includes(file.status)
      ? [{ label: t('common.pause'), icon: Pause, onselect: () => queue.run(() => api.downloads.pause(file.id)) }]
      : []),
    ...(file.status === 'failed' || file.status === 'skipped'
      ? [{ label: t('common.retry'), icon: RotateCcw, onselect: () => queue.run(() => api.downloads.retry(file.id)) }]
      : []),
    { label: t('common.delete'), icon: Trash2, danger: true, onselect: () => deleteFile(file) },
  ])
</script>

<li class="grid grid-cols-[0.5rem_minmax(0,1fr)_auto_auto] items-center gap-x-3 py-2 pr-1 pl-1">
  <span class="size-1.5 rounded-full {toneFill[tone]}" aria-hidden="true"></span>

  <div class="min-w-0">
    <p class="truncate text-small" title={file.url}>{displayName(file)}</p>
    <p class="mt-px flex min-w-0 flex-wrap gap-x-3 text-xs text-fg-faint">
      <span class={toneText[tone]}>{t(`status.download.${file.status}`)}</span>
      {#if retryIn !== null && file.status === 'queued'}
        <span class="num text-warn">{t('queue.retryIn', { duration: formatDuration(retryIn) })}</span>
      {/if}
      {#if file.attempts > 1}<span class="num">{t('queue.attempt', { count: file.attempts })}</span>{/if}
      {#if file.host}<span>{file.host}</span>{/if}
      {#if via}<span>{via}</span>{/if}
      {#if file.checksum}
        <span
          class="flex items-center gap-1 {file.checksum.status === 'ok' ? 'text-ok' : 'text-bad'}"
          title={file.checksum.algo.toUpperCase()}
        >
          {#if file.checksum.status === 'ok'}
            <ShieldCheck size={12} aria-hidden="true" />{t('queue.checksumOk')}
          {:else}
            <ShieldAlert size={12} aria-hidden="true" />{t('queue.checksumBad')}
          {/if}
        </span>
      {/if}
    </p>
    {#if file.error}
      <p class="mt-0.5 text-xs break-words text-bad">{file.error}</p>
    {/if}
  </div>

  <div class="w-28 text-right num text-xs text-fg-muted sm:w-52">
    {#if transferring}
      <div class="flex items-center justify-end gap-3">
        <span class="hidden sm:inline">{formatPercent(ratio)}</span>
        <span class="text-fg">{formatSpeed(file.speed)}</span>
        <span class="hidden w-14 sm:inline">{formatDuration(file.eta)}</span>
      </div>
      {#if file.map}
        <div class="mt-1.5 overflow-hidden rounded-full">
          <SegmentStrip
            map={file.map}
            heads={file.heads ?? []}
            label={file.heads?.length
              ? t('dashboard.segments', { count: file.heads.length })
              : t('dashboard.segmentsIdle')}
          />
        </div>
      {:else}
        <ProgressBar value={ratio} size="xs" class="mt-1" label={displayName(file)} />
      {/if}
    {:else}
      {formatBytes(file.size)}
    {/if}
  </div>

  <Menu {items} />
</li>
