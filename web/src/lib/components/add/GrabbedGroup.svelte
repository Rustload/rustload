<script lang="ts">
  import X from '@lucide/svelte/icons/x'
  import { t } from '$lib/i18n/index.svelte'
  import { grabber } from '$lib/stores/grabber.svelte'
  import { formatBytes, hostOf } from '$lib/utils/format'
  import type { GrabbedLink, LinkGroup } from '$lib/utils/grouping'
  import DisclosureButton from '../ui/DisclosureButton.svelte'
  import IconButton from '../ui/IconButton.svelte'
  import Spinner from '../ui/Spinner.svelte'
  import GrabbedLinkRow from './GrabbedLinkRow.svelte'
  import LinkLamp from './LinkLamp.svelte'

  /**
   * One package of the link grabber. A link alone is a single row whose title is the package name;
   * several links get a folded row that sums them up and lists them when opened.
   */
  type Props = {
    group: LinkGroup
    name: string
    /** Shown while the server hasn't given a name yet (magnets, playlists). */
    placeholder: string
    files: number | null
    size: number | null
    onrename: (name: string) => void
    onremove: (urls: Set<string>) => void
    onscan: (url: string) => void
  }

  let { group, name, placeholder, files, size, onrename, onremove, onscan }: Props = $props()

  let expanded = $state(false)
  const linksId = $props.id()

  function unavailable(link: GrabbedLink): boolean {
    const info = link.info
    return !!info && info.handler.type !== 'page' && (info.status === 'offline' || info.status === 'unsupported')
  }

  const summary = $derived.by(() => {
    const s = { checking: 0, online: 0, offline: 0, duplicates: 0 }
    for (const link of group.links) {
      if (grabber.results.get(link.url)?.state === 'checking') s.checking++
      else if (unavailable(link)) s.offline++
      else if (link.info?.status === 'online') s.online++
      if (link.info?.duplicate) s.duplicates++
    }
    return s
  })
  const lamp = $derived(
    summary.checking
      ? 'checking'
      : summary.offline
        ? 'offline'
        : summary.online === group.links.length
          ? 'online'
          : 'unknown',
  )
  const lampLabel = $derived.by(() => {
    if (lamp === 'checking') return t('grabber.checking')
    if (lamp !== 'unknown') return t(`grabber.status.${lamp}`)
    const unchecked = group.links.some((l) => grabber.results.get(l.url)?.state !== 'done')
    return t(unchecked ? 'grabber.status.notChecked' : 'grabber.status.unknown')
  })
  const hosts = $derived([...new Set(group.links.map((l) => l.info?.host ?? hostOf(l.url)).filter(Boolean))])
</script>

{#snippet nameField()}
  <input
    value={name}
    {placeholder}
    oninput={(e) => onrename(e.currentTarget.value)}
    onkeydown={(e) => {
      // Enter confirms the name; it must not submit the whole form.
      if (e.key === 'Enter') {
        e.preventDefault()
        e.currentTarget.blur()
      }
    }}
    aria-label={t('grabber.packageName')}
    title={t('grabber.packageName')}
    spellcheck={false}
    class="-ml-1.5 w-full min-w-0 truncate rounded-chip bg-transparent px-1.5 py-px font-medium text-fg
      placeholder:text-fg-faint hover:bg-sunken focus:bg-sunken focus:outline-none focus-visible:ring-2
      focus-visible:ring-info/40"
  />
{/snippet}

{#if group.links.length === 1}
  {@const link = group.links[0]}
  <GrabbedLinkRow
    {link}
    check={grabber.results.get(link.url)}
    scanning={grabber.scanning.has(link.url)}
    title={nameField}
    onremove={() => onremove(new Set([link.url]))}
    onscan={() => onscan(link.url)}
  />
{:else}
  <li>
    <div class="group/row flex items-center gap-3 py-2 pr-3 pl-5">
      <LinkLamp state={lamp} label={lampLabel} />
      <div class="min-w-0 flex-1">
        {@render nameField()}
        <p class="mt-px flex min-w-0 flex-wrap gap-x-3 text-xs text-fg-faint">
          {#if files !== null}<span class="num">{t('grabber.folderCount', { count: files })}</span>{/if}
          {#if hosts.length}<span class="truncate">{hosts.join(', ')}</span>{/if}
          {#if summary.offline}
            <span class="text-bad">{t('grabber.offline', { count: summary.offline })}</span>
          {/if}
          {#if summary.duplicates}
            <span class="text-warn">{t('grabber.duplicates', { count: summary.duplicates })}</span>
          {/if}
        </p>
      </div>
      <span class="flex w-20 shrink-0 justify-end num text-xs text-fg-muted">
        {#if summary.checking && size === null}
          <Spinner size={13} class="text-fg-faint" />
        {:else}
          {formatBytes(size)}
        {/if}
      </span>
      <DisclosureButton bind:expanded controls={linksId} />
      <IconButton
        icon={X}
        label={t('grabber.removeLink', { name: name || placeholder })}
        size="sm"
        class="shrink-0 opacity-60 group-hover/row:opacity-100 focus-visible:opacity-100"
        onclick={() => onremove(new Set(group.links.map((l) => l.url)))}
      />
    </div>
    {#if expanded}
      <ul id={linksId} class="pb-1.5">
        {#each group.links as link (link.url)}
          <GrabbedLinkRow
            {link}
            check={grabber.results.get(link.url)}
            scanning={grabber.scanning.has(link.url)}
            nested
            onremove={() => onremove(new Set([link.url]))}
            onscan={() => onscan(link.url)}
          />
        {/each}
      </ul>
    {/if}
  </li>
{/if}
