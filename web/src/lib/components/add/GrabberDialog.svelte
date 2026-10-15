<script lang="ts">
  import FileUp from '@lucide/svelte/icons/file-up'
  import Magnet from '@lucide/svelte/icons/magnet'
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert'
  import X from '@lucide/svelte/icons/x'
  import { untrack } from 'svelte'
  import { SvelteMap } from 'svelte/reactivity'
  import { api, type AddOptions as AddRequestOptions, type AddRequest, type MediaOptions } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { accounts } from '$lib/stores/catalog.svelte'
  import { grabber } from '$lib/stores/grabber.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { errorMessage, toasts } from '$lib/stores/toasts.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { formatBytes } from '$lib/utils/format'
  import { groupLinks, type GrabbedLink, type LinkGroup } from '$lib/utils/grouping'
  import { isTorrentFile, parseLinks } from '$lib/utils/links'
  import type { PackageOptions } from '../queue/PackageOptionsFields.svelte'
  import Alert from '../ui/Alert.svelte'
  import Button from '../ui/Button.svelte'
  import Checkbox from '../ui/Checkbox.svelte'
  import Dialog from '../ui/Dialog.svelte'
  import IconButton from '../ui/IconButton.svelte'
  import TextArea from '../ui/TextArea.svelte'
  import Toggle from '../ui/Toggle.svelte'
  import AddOptions from './AddOptions.svelte'
  import GrabbedGroup from './GrabbedGroup.svelte'

  /** Packages per `POST /api/packages/batch` request (server maximum). */
  const BATCH_LIMIT = 100

  let text = $state('')
  let files = $state<File[]>([])
  let paused = $state(false)
  let options = $state<PackageOptions>(defaults())
  let media = $state<MediaOptions>(mediaDefaults())
  let single = $state(false)
  const names = new SvelteMap<string, string>()
  let submitting = $state(false)
  let error = $state<string | null>(null)
  let dragging = $state(false)
  /** Links from another page wait for the user's go-ahead before the server contacts them. */
  let checking = $state(true)
  let fileInput: HTMLInputElement | undefined = $state()

  const parsed = $derived(parseLinks(text))
  const links: GrabbedLink[] = $derived(
    [
      ...parsed.links.map((url) => ({ url, kind: 'http' as const })),
      ...parsed.magnets.map((url) => ({ url, kind: 'magnet' as const })),
    ].map((link) => {
      const check = grabber.results.get(link.url)
      return { ...link, info: check?.state === 'done' ? check.info : null }
    }),
  )
  const groups = $derived(groupLinks(links, single ? 'single' : 'smart'))
  const total = $derived(links.length + files.length)
  const packageCount = $derived(groups.length + files.length)
  const hasMedia = $derived(links.some((l) => l.info?.handler.type === 'media'))
  const needsMagnets = $derived(parsed.magnets.length + files.length > 0)

  const counts = $derived.by(() => {
    const c = { checking: 0, online: 0, offline: 0, duplicates: 0 }
    for (const link of links) {
      const check = grabber.results.get(link.url)
      if (check?.state === 'checking' || (!check && checking)) c.checking++
      else if (link.info?.status === 'online') c.online++
      else if (isUnavailable(link)) c.offline++
      if (link.info?.duplicate) c.duplicates++
    }
    return c
  })

  /** Offline, or no account can handle it. Web pages are not unavailable: they can be scanned. */
  function isUnavailable(link: GrabbedLink): boolean {
    const info = link.info
    if (!info || info.handler.type === 'page') return false
    return info.status === 'offline' || info.status === 'unsupported'
  }

  function defaults(): PackageOptions {
    const s = settings.current
    return {
      category_id: null,
      create_subfolder: s?.storage.create_subfolder ?? true,
      extract: s?.postprocess.extract ?? true,
      delete_archives: s?.postprocess.delete_archives ?? true,
      archive_password: '',
    }
  }

  function mediaDefaults(): MediaOptions {
    return structuredClone(
      $state.snapshot(settings.current?.media?.defaults) ?? {
        quality: 'best',
        audio_format: 'best',
        container: 'auto',
        subtitles: false,
        subtitle_langs: 'en',
        embed_metadata: true,
        embed_thumbnail: true,
        sponsorblock: 'off',
      },
    )
  }

  // Reset everything each time the dialog opens, prefilled with pasted text / dropped files.
  $effect(() => {
    if (!ui.addOpen) return
    untrack(() => {
      grabber.reset()
      text = ui.addText
      files = ui.addFiles
      paused = settings.current ? !settings.current.downloads.auto_start : false
      options = defaults()
      media = mediaDefaults()
      single = false
      names.clear()
      error = null
      checking = ui.addTrusted
    })
  })

  // Check links shortly after the text stops changing.
  $effect(() => {
    if (!ui.addOpen || !checking) return
    const value = text
    const timer = setTimeout(() => grabber.check(value), 250)
    return () => clearTimeout(timer)
  })

  /** Rewrites the text without the given links (one link per line). */
  function removeLinks(urls: Set<string>, replacement: string[] = []) {
    const kept = [...parsed.links, ...parsed.magnets].filter((url) => !urls.has(url))
    text = [...kept, ...replacement.filter((url) => !kept.includes(url))].join('\n')
  }

  function removeWhere(predicate: (link: GrabbedLink) => boolean) {
    removeLinks(new Set(links.filter(predicate).map((l) => l.url)))
  }

  async function scan(url: string) {
    try {
      const result = await grabber.scan(url)
      // The dialog was closed, or the page removed from the list, while it was being read.
      if (result.stale || !ui.addOpen || !parsed.links.includes(url)) return
      if (result.found === null) toasts.push('warning', t('grabber.scanFailed'))
      else if (result.found.length) {
        removeLinks(new Set([url]), result.found)
        toasts.success(t('grabber.foundLinks', { count: result.found.length }))
      }
    } catch (err) {
      toasts.error(err)
    }
  }

  function addFiles(list: FileList | File[] | null | undefined) {
    const known = new Set(files.map((f) => f.name))
    const added: File[] = []
    for (const file of [...(list ?? [])].filter(isTorrentFile)) {
      if (known.has(file.name)) continue
      known.add(file.name)
      added.push(file)
    }
    files = [...files, ...added]
  }

  function ondrop(event: DragEvent) {
    event.preventDefault()
    dragging = false
    addFiles(event.dataTransfer?.files)
    const dropped = event.dataTransfer?.getData('text/plain')
    if (dropped) text = text ? `${text}\n${dropped}` : dropped
  }

  function groupName(group: LinkGroup): string {
    return names.get(group.key) ?? group.name
  }

  /** Shown while a magnet or a playlist has no name yet; the server picks one when it's added. */
  function placeholderName(group: LinkGroup): string {
    return group.kind === 'magnet' ? t('source.magnet') : group.kind === 'playlist' ? t('grabber.playlist') : ''
  }

  /** The name sent to the server: an edited one, or ours once every file name is known. */
  function requestName(group: LinkGroup): string | undefined {
    const edited = names.get(group.key)?.trim()
    if (edited) return edited
    const settled = group.links.every((l) => l.info !== null)
    return settled && group.kind !== 'magnet' && group.name ? group.name : undefined
  }

  /** Files the group will create: folders count their content; magnets are unknown until processed. */
  function groupFiles(group: LinkGroup): number | null {
    if (group.kind === 'magnet') return null
    return group.links.reduce((sum, l) => sum + (l.info?.folder?.files ?? l.info?.media?.playlist?.count ?? 1), 0)
  }

  function groupSize(group: LinkGroup): number | null {
    let sum = 0
    for (const link of group.links) {
      const size = link.info?.folder?.size ?? link.info?.size
      if (size == null) return null
      sum += size
    }
    return sum
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    if (total === 0) return
    submitting = true
    error = null
    const common: AddRequestOptions = {
      category_id: options.category_id,
      create_subfolder: options.create_subfolder,
      extract: options.extract,
      delete_archives: options.delete_archives,
      archive_password: options.archive_password.trim() || undefined,
      paused,
      media: hasMedia ? $state.snapshot(media) : undefined,
    }
    // Sent in chunks the server accepts; whatever was added leaves the form at once, so trying
    // again after an error never adds it twice.
    let added = 0
    // Removing added links re-groups the rest: work from the plan as it was when submitted.
    const planned = [...groups]
    try {
      for (let i = 0; i < planned.length; i += BATCH_LIMIT) {
        const chunk = planned.slice(i, i + BATCH_LIMIT)
        const requests: AddRequest[] = chunk.map((group) => ({
          ...common,
          name: requestName(group),
          links: group.links.map((l) => l.url).join('\n'),
        }))
        added += (await api.packages.addBatch(requests)).packages.length
        const urls = new Set(chunk.flatMap((group) => group.links.map((l) => l.url)))
        removeLinks(urls)
        grabber.forget(urls)
      }
      if (files.length) {
        added += (await api.packages.addTorrents(files, common)).packages.length
        files = []
      }
      toasts.success(t('add.added', { count: added }))
      ui.addOpen = false
    } catch (err) {
      error = added ? `${t('add.added', { count: added })}. ${errorMessage(err)}` : errorMessage(err)
    } finally {
      submitting = false
      if (added) queue.invalidate()
    }
  }
</script>

<Dialog bind:open={ui.addOpen} title={t('grabber.title')} size="lg" fill initialFocus="textarea">
  <form
    id="grabber-form"
    class="flex flex-col sm:min-h-0 sm:flex-1"
    onsubmit={submit}
    ondragover={(e) => {
      e.preventDefault()
      dragging = true
    }}
    ondragleave={(e) => {
      if (!e.currentTarget.contains(e.relatedTarget as Node | null)) dragging = false
    }}
    {ondrop}
  >
    <div class="shrink-0 space-y-3 px-5 pb-4">
      <div class="relative">
        <label for="grabber-links" class="sr-only">{t('grabber.input')}</label>
        <TextArea
          id="grabber-links"
          bind:value={text}
          mono
          rows={4}
          spellcheck={false}
          autocomplete="off"
          placeholder={t('grabber.inputPlaceholder')}
          class="[field-sizing:content] max-h-40 min-h-24 [overflow-wrap:anywhere]"
        />
        {#if dragging}
          <div
            class="pointer-events-none absolute inset-0 grid place-items-center rounded-control border-2 border-dashed
              border-info bg-info-wash font-medium text-fg"
          >
            {t('add.dropActive')}
          </div>
        {/if}
      </div>

      <p class="flex flex-wrap items-center gap-x-1.5 text-small text-fg-muted">
        <FileUp size={14} class="text-fg-faint" aria-hidden="true" />
        {t('add.drop')}
        <button
          type="button"
          class="font-medium text-fg underline underline-offset-2"
          onclick={() => fileInput?.click()}
        >
          {t('common.browse')}
        </button>
        <input
          bind:this={fileInput}
          type="file"
          accept=".torrent,application/x-bittorrent"
          multiple
          class="hidden"
          onchange={(e) => {
            addFiles(e.currentTarget.files)
            e.currentTarget.value = ''
          }}
        />
      </p>

      {#if total === 0}
        <p class="text-small text-fg-faint">{t('grabber.hint')}</p>
      {/if}

      {#if needsMagnets && !accounts.magnetCapable}
        <Alert tone="warn" icon={TriangleAlert} class="text-small">{t('add.noMagnetAccount')}</Alert>
      {/if}
    </div>

    {#if total > 0}
      <div class="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-2 border-t border-line px-5 py-2.5">
        <p class="flex min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-1 text-small text-fg-muted">
          <span class="font-medium text-fg">{t('queue.count', { count: packageCount })}</span>
          <span class="num">{t('grabber.linkCount', { count: total })}</span>
          {#if counts.online}
            <span class="flex items-center gap-1.5 num">
              <span class="size-1.5 rounded-full bg-ok" aria-hidden="true"></span>
              {t('grabber.online', { count: counts.online })}
            </span>
          {/if}
          {#if counts.checking}
            <span class="flex items-center gap-1.5 num">
              <span class="size-1.5 animate-pulse rounded-full bg-fg-faint" aria-hidden="true"></span>
              {t('grabber.checkingCount', { count: counts.checking })}
            </span>
          {/if}
          {#if counts.offline}
            <span class="flex items-center gap-1.5 num text-bad">
              <span class="size-1.5 rounded-full bg-bad" aria-hidden="true"></span>
              {t('grabber.offline', { count: counts.offline })}
              <button
                type="button"
                class="font-medium underline underline-offset-2"
                aria-label={t('grabber.removeOffline', { count: counts.offline })}
                onclick={() => removeWhere(isUnavailable)}
              >
                {t('common.remove')}
              </button>
            </span>
          {/if}
          {#if counts.duplicates}
            <span class="flex items-center gap-1.5 num text-warn">
              {t('grabber.duplicates', { count: counts.duplicates })}
              <button
                type="button"
                class="font-medium underline underline-offset-2"
                aria-label={t('grabber.removeDuplicates', { count: counts.duplicates })}
                onclick={() => removeWhere((l) => !!l.info?.duplicate)}
              >
                {t('common.remove')}
              </button>
            </span>
          {/if}
        </p>
        <!-- Announced once, when every link has been checked, instead of at each change. -->
        <p class="sr-only" aria-live="polite">
          {#if checking && counts.checking === 0}
            {t('grabber.checkedSummary', { count: total, online: counts.online, offline: counts.offline })}
          {/if}
        </p>
        {#if links.length > 1}
          <Checkbox bind:checked={single} label={t('grabber.singlePackage')} class="text-small" />
        {/if}
      </div>

      {#if !checking}
        <div class="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-2 border-t border-line bg-warn-wash px-5 py-3">
          <p class="min-w-0 flex-1 basis-64 text-small text-warn">{t('grabber.untrusted')}</p>
          <Button size="sm" onclick={() => (checking = true)}>{t('grabber.checkNow')}</Button>
        </div>
      {/if}

      <ul class="divide-y divide-line border-t border-line sm:min-h-0 sm:flex-1 sm:overflow-y-auto">
        {#each groups as group (group.key)}
          <GrabbedGroup
            {group}
            name={groupName(group)}
            placeholder={placeholderName(group)}
            files={groupFiles(group)}
            size={groupSize(group)}
            onrename={(name) => names.set(group.key, name)}
            onremove={(urls) => removeLinks(urls)}
            onscan={scan}
          />
        {/each}
        {#each files as file (file.name)}
          <li class="group/row flex items-center gap-3 py-2 pr-3 pl-5">
            <Magnet size={14} class="-mx-[3px] shrink-0 text-fg-faint" aria-hidden="true" />
            <div class="min-w-0 flex-1">
              <p class="truncate font-medium">{file.name}</p>
              <p class="mt-px text-xs text-fg-faint">{t('grabber.torrentFile')}</p>
            </div>
            <span class="w-20 shrink-0 text-right num text-xs text-fg-muted">{formatBytes(file.size)}</span>
            <span class="w-7 shrink-0" aria-hidden="true"></span>
            <IconButton
              icon={X}
              label={t('add.removeFile', { name: file.name })}
              size="sm"
              class="shrink-0 opacity-60 group-hover/row:opacity-100 focus-visible:opacity-100"
              onclick={() => (files = files.filter((f) => f !== file))}
            />
          </li>
        {/each}
      </ul>
    {/if}

    <AddOptions bind:options bind:media {hasMedia} />
  </form>

  {#snippet footer()}
    <Toggle bind:checked={paused} label={t('add.paused')} class="mr-auto" />
    {#if error}
      <p role="alert" class="w-full text-small text-bad sm:order-first">{error}</p>
    {/if}
    <Button onclick={() => (ui.addOpen = false)}>{t('common.cancel')}</Button>
    <Button type="submit" form="grabber-form" variant="primary" loading={submitting} disabled={total === 0}>
      {paused ? t('grabber.submitPaused', { count: packageCount }) : t('grabber.submit', { count: packageCount })}
    </Button>
  {/snippet}
</Dialog>
