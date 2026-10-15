<script lang="ts">
  import ScanSearch from '@lucide/svelte/icons/scan-search'
  import X from '@lucide/svelte/icons/x'
  import type { Snippet } from 'svelte'
  import { t } from '$lib/i18n/index.svelte'
  import type { CheckState } from '$lib/stores/grabber.svelte'
  import { formatBytes, formatDuration, hostOf } from '$lib/utils/format'
  import { linkName, type GrabbedLink } from '$lib/utils/grouping'
  import { linkTone, toneText } from '$lib/utils/status'
  import Button from '../ui/Button.svelte'
  import DisclosureButton from '../ui/DisclosureButton.svelte'
  import IconButton from '../ui/IconButton.svelte'
  import Spinner from '../ui/Spinner.svelte'
  import LinkLamp from './LinkLamp.svelte'

  type Props = {
    link: GrabbedLink
    check: CheckState | undefined
    scanning: boolean
    /** Replaces the link name, e.g. with the package name field when the link is a package alone. */
    title?: Snippet
    /** One of several links of a package, shown under its row. */
    nested?: boolean
    onremove: () => void
    onscan: () => void
  }

  let { link, check, scanning, title, nested = false, onremove, onscan }: Props = $props()

  let expanded = $state(false)
  const entriesId = $props.id()

  const info = $derived(link.info)
  const checking = $derived(check?.state === 'checking')
  const isPage = $derived(info?.handler.type === 'page')
  // A web page isn't a failure: it's something to scan for links.
  const lamp = $derived.by(() => {
    if (checking) return 'checking'
    if (!info || isPage) return 'unknown'
    return info.status
  })
  const lampLabel = $derived.by(() => {
    if (lamp === 'checking') return t('grabber.checking')
    if (!check || check.state === 'failed') return t('grabber.status.notChecked')
    if (isPage) return t('grabber.handler.page')
    if (link.kind === 'magnet' && lamp === 'unknown') return t('grabber.status.magnet')
    return t(`grabber.status.${lamp}`)
  })

  const handler = $derived.by(() => {
    const h = info?.handler
    if (!h) return null
    switch (h.type) {
      case 'debrid':
      case 'magnet':
        return h.id ? t('grabber.via', { name: h.name }) : null
      case 'direct':
        return t('grabber.handler.direct')
      case 'page':
        return t('grabber.handler.page')
      case 'playlist':
        return t('grabber.handler.playlist')
      case 'media':
        return h.id === 'hls' ? t('grabber.handler.hls') : h.name
      default:
        return h.name
    }
  })
  const host = $derived(info?.host ?? (link.kind === 'http' ? hostOf(link.url) : null))
  const media = $derived(info?.media ?? null)
  const folder = $derived(info?.folder ?? null)
  const size = $derived(folder?.size ?? info?.size ?? null)
  const noLinksFound = $derived(isPage && info?.found !== null && info?.found.length === 0)
</script>

<li class="group/row py-2 pr-3 {nested ? 'pl-10' : 'pl-5'}">
  <div class="flex items-center gap-3">
    <LinkLamp state={lamp} label={lampLabel} />

    {#if media?.thumbnail}
      <span class="relative hidden h-9 w-16 shrink-0 overflow-hidden rounded-chip bg-sunken sm:block">
        <img src={media.thumbnail} alt="" loading="lazy" referrerpolicy="no-referrer" class="size-full object-cover" />
        {#if media.duration_secs}
          <span class="absolute right-0.5 bottom-0.5 rounded-chip bg-black/70 px-1 num text-[0.625rem] text-white">
            {formatDuration(media.duration_secs)}
          </span>
        {/if}
      </span>
    {/if}

    <div class="min-w-0 flex-1">
      {#if title}
        {@render title()}
      {:else}
        <p class="truncate text-small {checking ? 'text-fg-muted' : 'text-fg'}" title={link.url}>
          {linkName(link)}
        </p>
      {/if}
      <p class="mt-px flex min-w-0 flex-wrap gap-x-3 text-xs text-fg-faint">
        {#if host}<span class="truncate">{host}</span>{/if}
        {#if handler}<span>{handler}</span>{/if}
        {#if media?.uploader}<span class="truncate">{media.uploader}</span>{/if}
        {#if media?.playlist}<span class="num">{t('grabber.playlistCount', { count: media.playlist.count })}</span>{/if}
        {#if folder}<span class="num">{t('grabber.folderCount', { count: folder.files })}</span>{/if}
        {#if !isPage && (info?.status === 'offline' || info?.status === 'unsupported')}
          <span class={toneText[linkTone[info.status]]}>
            {info.error ?? t(`grabber.status.${info.status}`)}
          </span>
        {/if}
        {#if info?.duplicate}
          <span class="text-warn">
            {t(info.duplicate.location === 'queue' ? 'grabber.dupQueue' : 'grabber.dupHistory', {
              name: info.duplicate.package_name,
            })}
          </span>
        {/if}
        {#if noLinksFound}<span class="text-warn">{t('grabber.noLinksFound')}</span>{/if}
      </p>
    </div>

    {#if isPage && !noLinksFound}
      <Button size="sm" icon={scanning ? undefined : ScanSearch} loading={scanning} onclick={onscan}>
        {t('grabber.findLinks')}
      </Button>
    {:else}
      <span class="flex w-20 shrink-0 justify-end num text-xs text-fg-muted">
        {#if checking}
          <Spinner size={13} class="text-fg-faint" />
        {:else if size !== null}
          {formatBytes(size)}
        {:else if media?.duration_secs}
          {formatDuration(media.duration_secs)}
        {:else}
          <span class="text-fg-faint">—</span>
        {/if}
      </span>
    {/if}

    {#if folder && folder.entries.length}
      <DisclosureButton bind:expanded controls={entriesId} />
    {:else}
      <span class="w-7 shrink-0" aria-hidden="true"></span>
    {/if}

    <IconButton
      icon={X}
      label={t('grabber.removeLink', { name: linkName(link) })}
      size="sm"
      class="shrink-0 opacity-60 group-hover/row:opacity-100 focus-visible:opacity-100"
      onclick={onremove}
    />
  </div>

  {#if expanded && folder}
    <ul id={entriesId} class="mt-1.5 ml-5 space-y-0.5 border-l border-line pl-3 text-xs text-fg-muted">
      {#each folder.entries as entry, i (i)}
        <li class="flex gap-3">
          <span class="min-w-0 flex-1 truncate">{entry.rel_dir ? `${entry.rel_dir}/` : ''}{entry.name}</span>
          {#if entry.size !== null}<span class="shrink-0 num text-fg-faint">{formatBytes(entry.size)}</span>{/if}
        </li>
      {/each}
      {#if folder.files > folder.entries.length}
        <li class="text-fg-faint">{t('grabber.moreFiles', { count: folder.files - folder.entries.length })}</li>
      {/if}
    </ul>
  {/if}
</li>
