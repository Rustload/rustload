<script lang="ts">
  import Archive from '@lucide/svelte/icons/archive'
  import FileUp from '@lucide/svelte/icons/file-up'
  import Folder from '@lucide/svelte/icons/folder'
  import ListVideo from '@lucide/svelte/icons/list-video'
  import Magnet from '@lucide/svelte/icons/magnet'
  import Package from '@lucide/svelte/icons/package'
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert'
  import X from '@lucide/svelte/icons/x'
  import { untrack } from 'svelte'
  import { SvelteMap } from 'svelte/reactivity'
  import { api, type AddOptions, type AddRequest, type MediaOptions } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { accounts } from '$lib/stores/catalog.svelte'
  import { grabber } from '$lib/stores/grabber.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { errorMessage, toasts } from '$lib/stores/toasts.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { formatBytes } from '$lib/utils/format'
  import { groupLinks, type GrabbedLink, type GroupKind, type GroupMode, type LinkGroup } from '$lib/utils/grouping'
  import { isTorrentFile, parseLinks } from '$lib/utils/links'
  import PackageOptionsFields, { type PackageOptions } from '../queue/PackageOptionsFields.svelte'
  import Alert from '../ui/Alert.svelte'
  import Button from '../ui/Button.svelte'
  import Dialog from '../ui/Dialog.svelte'
  import IconButton from '../ui/IconButton.svelte'
  import Segmented from '../ui/Segmented.svelte'
  import TextArea from '../ui/TextArea.svelte'
  import Toggle from '../ui/Toggle.svelte'
  import GrabbedLinkRow from './GrabbedLinkRow.svelte'
  import MediaOptionsFields from './MediaOptionsFields.svelte'

  /** Packages per `POST /api/packages/batch` request (server maximum). */
  const BATCH_LIMIT = 100

  let text = $state('')
  let files = $state<File[]>([])
  let paused = $state(false)
  let options = $state<PackageOptions>(defaults())
  let media = $state<MediaOptions>(mediaDefaults())
  let mode = $state<GroupMode>('smart')
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
  const groups = $derived(groupLinks(links, mode))
  const total = $derived(links.length + files.length)
  const packageCount = $derived(groups.length + files.length)
  const hasMedia = $derived(links.some((l) => l.info?.handler.type === 'media'))
  const needsMagnets = $derived(parsed.magnets.length + files.length > 0)

  const counts = $derived.by(() => {
    const c = { checking: 0, online: 0, offline: 0, unknown: 0, duplicates: 0 }
    for (const link of links) {
      const check = grabber.results.get(link.url)
      if (check?.state === 'checking' || (!check && checking)) c.checking++
      else if (link.info?.status === 'online') c.online++
      else if (isUnavailable(link)) c.offline++
      else c.unknown++
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
      mode = 'smart'
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

  const KIND_ICON: Record<GroupKind, typeof Package> = {
    files: Package,
    archive: Archive,
    folder: Folder,
    playlist: ListVideo,
    magnet: Magnet,
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
    const common: AddOptions = {
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

<Dialog bind:open={ui.addOpen} title={t('grabber.title')} size="xl" fill initialFocus="textarea">
  <!-- Phones stack input, preview, then options; wider screens keep the preview beside the form. -->
  <form
    id="grabber-form"
    class="grid grid-cols-[minmax(0,1fr)] lg:h-full lg:grid-cols-[21rem_minmax(0,1fr)]"
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
    <div class="contents lg:block lg:space-y-5 lg:overflow-y-auto lg:border-r lg:border-line lg:px-5 lg:pb-5">
      <div class="order-1 space-y-4 px-5 pb-5 lg:p-0">
        <div>
          <label for="grabber-links" class="mb-1.5 block font-medium">{t('grabber.input')}</label>
          <TextArea
            id="grabber-links"
            bind:value={text}
            mono
            rows={7}
            spellcheck={false}
            autocomplete="off"
            placeholder={t('grabber.inputPlaceholder')}
            class="min-h-32 [overflow-wrap:anywhere]"
          />
          <div
            role="group"
            aria-label={t('grabber.dropZone')}
            class="mt-2 flex flex-wrap items-center gap-1.5 rounded-control border border-dashed px-3 py-2.5 text-small
            transition-colors {dragging ? 'border-info bg-info-wash text-fg' : 'border-line-strong text-fg-muted'}"
          >
            <FileUp size={15} class="text-fg-faint" aria-hidden="true" />
            {#if dragging}
              {t('add.dropActive')}
            {:else}
              {t('add.drop')}
              <button
                type="button"
                class="font-medium text-fg underline underline-offset-2"
                onclick={() => fileInput?.click()}
              >
                {t('common.browse')}
              </button>
            {/if}
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
          </div>
        </div>

        {#if needsMagnets && !accounts.magnetCapable}
          <Alert tone="warn" icon={TriangleAlert} class="text-small">{t('add.noMagnetAccount')}</Alert>
        {/if}
      </div>

      <div class="order-3 space-y-5 border-t border-line px-5 py-5 lg:border-0 lg:p-0">
        <PackageOptionsFields bind:options />

        {#if hasMedia}
          <section aria-labelledby="grabber-media" class="space-y-3 border-t border-line pt-5">
            <h3 id="grabber-media" class="font-semibold heading">{t('media.title')}</h3>
            <MediaOptionsFields bind:options={media} />
          </section>
        {/if}

        <Toggle bind:checked={paused} label={t('add.paused')} />
      </div>
    </div>

    <div class="order-2 flex min-h-0 flex-col border-t border-line lg:border-t-0">
      <div class="flex min-h-12 flex-wrap items-center gap-x-4 gap-y-2 border-b border-line px-5 py-2">
        <p class="flex flex-wrap items-baseline gap-x-4 gap-y-1 num text-small text-fg-muted">
          {#if total === 0}
            <span class="text-fg-faint">{t('add.nothingDetected')}</span>
          {:else}
            <span class="font-medium text-fg">{t('grabber.linkCount', { count: total })}</span>
            {#if counts.online}
              <span class="flex items-center gap-1.5">
                <span class="size-1.5 rounded-full bg-ok" aria-hidden="true"></span>{t('grabber.online', {
                  count: counts.online,
                })}
              </span>
            {/if}
            {#if counts.offline}
              <span class="flex items-center gap-1.5 text-bad">
                <span class="size-1.5 rounded-full bg-bad" aria-hidden="true"></span>{t('grabber.offline', {
                  count: counts.offline,
                })}
              </span>
            {/if}
            {#if counts.checking}
              <span class="flex items-center gap-1.5">
                <span class="size-1.5 animate-pulse rounded-full bg-fg-faint" aria-hidden="true"></span>{t(
                  'grabber.checkingCount',
                  { count: counts.checking },
                )}
              </span>
            {/if}
            {#if counts.duplicates}
              <span class="text-warn">{t('grabber.duplicates', { count: counts.duplicates })}</span>
            {/if}
          {/if}
        </p>
        <!-- Announced once, when every link has been checked, instead of at each change. -->
        <p class="sr-only" aria-live="polite">
          {#if checking && total > 0 && counts.checking === 0}
            {t('grabber.checkedSummary', { count: total, online: counts.online, offline: counts.offline })}
          {/if}
        </p>
        {#if links.length > 1}
          <Segmented
            class="ml-auto"
            label={t('grabber.grouping')}
            bind:value={mode}
            options={[
              { value: 'smart', label: t('grabber.groupSmart') },
              { value: 'single', label: t('grabber.groupSingle') },
            ]}
          />
        {/if}
      </div>

      <div class="min-h-0 flex-1 lg:overflow-y-auto">
        {#if !checking && total > 0}
          <div class="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-line bg-warn-wash px-5 py-3">
            <p class="min-w-0 flex-1 basis-64 text-small text-warn">{t('grabber.untrusted')}</p>
            <Button size="sm" onclick={() => (checking = true)}>{t('grabber.checkNow')}</Button>
          </div>
        {/if}
        {#if total === 0}
          <div class="mx-auto max-w-md px-6 py-12 lg:py-20">
            <h3 class="text-base font-semibold heading">{t('grabber.emptyTitle')}</h3>
            <p class="mt-1.5 text-fg-muted">{t('grabber.emptyBody')}</p>
            <dl class="mt-6 grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2.5 text-small">
              <dt class="text-fg-faint">{t('grabber.example.hosters')}</dt>
              <dd class="font-mono text-xs text-fg-muted">https://1fichier.com/?k3y…</dd>
              <dt class="text-fg-faint">{t('grabber.example.media')}</dt>
              <dd class="font-mono text-xs text-fg-muted">https://youtu.be/jNQXAC9IVRw</dd>
              <dt class="text-fg-faint">{t('grabber.example.magnets')}</dt>
              <dd class="font-mono text-xs text-fg-muted">magnet:?xt=urn:btih:…</dd>
              <dt class="text-fg-faint">{t('grabber.example.pages')}</dt>
              <dd class="font-mono text-xs text-fg-muted">https://example.org/downloads</dd>
            </dl>
          </div>
        {:else}
          {#each groups as group (group.key)}
            {@const Icon = KIND_ICON[group.kind]}
            <section class="border-b border-line">
              <header class="flex items-center gap-3 pt-3 pr-1.5 pb-1 pl-4">
                <Icon size={15} class="shrink-0 text-fg-faint" aria-hidden="true" />
                <input
                  value={groupName(group)}
                  placeholder={placeholderName(group)}
                  oninput={(e) => names.set(group.key, e.currentTarget.value)}
                  onkeydown={(e) => {
                    // Enter confirms the name; it must not submit the whole form.
                    if (e.key === 'Enter') {
                      e.preventDefault()
                      e.currentTarget.blur()
                    }
                  }}
                  aria-label={t('grabber.packageName')}
                  spellcheck={false}
                  class="-ml-1.5 min-w-0 flex-1 rounded-chip bg-transparent px-1.5 py-0.5 font-semibold heading
                    hover:bg-sunken focus:bg-sunken focus:outline-none focus-visible:ring-2 focus-visible:ring-info/40"
                />
                {#if groupFiles(group) !== null}
                  <span class="hidden shrink-0 num text-xs text-fg-muted sm:inline">
                    {t('queue.files', { count: groupFiles(group) ?? 0 })}
                  </span>
                {/if}
                <span class="w-24 shrink-0 text-right num text-xs text-fg-muted">{formatBytes(groupSize(group))}</span>
                <span class="w-[4.25rem] shrink-0" aria-hidden="true"></span>
              </header>
              <ul class="pb-1.5">
                {#each group.links as link (link.url)}
                  <GrabbedLinkRow
                    {link}
                    check={grabber.results.get(link.url)}
                    scanning={grabber.scanning.has(link.url)}
                    onremove={() => removeLinks(new Set([link.url]))}
                    onscan={() => scan(link.url)}
                  />
                {/each}
              </ul>
            </section>
          {/each}

          {#if files.length}
            <section class="border-b border-line">
              <header class="flex items-center gap-2.5 px-4 pt-3 pb-1.5">
                <Magnet size={15} class="text-fg-faint" aria-hidden="true" />
                <h3 class="flex-1 font-semibold heading">{t('grabber.torrents')}</h3>
              </header>
              <ul class="pb-1.5">
                {#each files as file (file.name)}
                  <li class="flex items-center gap-3 py-1.5 pr-1.5 pl-4 text-small">
                    <span class="size-2 shrink-0 rounded-full border border-fg-faint" aria-hidden="true"></span>
                    <span class="min-w-0 flex-1 truncate">{file.name}</span>
                    <span class="shrink-0 num text-xs text-fg-muted">{formatBytes(file.size)}</span>
                    <IconButton
                      icon={X}
                      label={t('add.removeFile', { name: file.name })}
                      size="sm"
                      onclick={() => (files = files.filter((f) => f !== file))}
                    />
                  </li>
                {/each}
              </ul>
            </section>
          {/if}
        {/if}
      </div>
    </div>
  </form>

  {#snippet footer()}
    {#if error}
      <p role="alert" class="mr-auto w-full text-small text-bad sm:w-auto">{error}</p>
    {/if}
    {#if counts.offline}
      <Button size="sm" variant="ghost" onclick={() => removeWhere(isUnavailable)}>
        {t('grabber.removeOffline', { count: counts.offline })}
      </Button>
    {/if}
    {#if counts.duplicates}
      <Button size="sm" variant="ghost" onclick={() => removeWhere((l) => !!l.info?.duplicate)}>
        {t('grabber.removeDuplicates', { count: counts.duplicates })}
      </Button>
    {/if}
    <span class="hidden flex-1 sm:block"></span>
    <Button onclick={() => (ui.addOpen = false)}>{t('common.cancel')}</Button>
    <Button type="submit" form="grabber-form" variant="primary" loading={submitting} disabled={total === 0}>
      {paused ? t('grabber.submitPaused', { count: packageCount }) : t('grabber.submit', { count: packageCount })}
    </Button>
  {/snippet}
</Dialog>
