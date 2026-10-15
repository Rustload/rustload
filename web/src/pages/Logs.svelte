<script lang="ts">
  import Download from '@lucide/svelte/icons/download'
  import Trash2 from '@lucide/svelte/icons/trash-2'
  import { onMount, tick } from 'svelte'
  import { api, type LogLevel } from '$lib/api'
  import Button from '$lib/components/ui/Button.svelte'
  import PageHeader from '$lib/components/ui/PageHeader.svelte'
  import SearchInput from '$lib/components/ui/SearchInput.svelte'
  import Segmented from '$lib/components/ui/Segmented.svelte'
  import Toggle from '$lib/components/ui/Toggle.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { logs } from '$lib/stores/logs.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { formatLogTime } from '$lib/utils/format'
  import { logTone, toneText } from '$lib/utils/status'

  const RANK: Record<LogLevel, number> = { ERROR: 0, WARN: 1, INFO: 2, DEBUG: 3 }

  let level = $state<LogLevel>('INFO')
  let query = $state('')
  let follow = $state(true)
  let viewport: HTMLDivElement | undefined = $state()

  onMount(() => {
    if (!logs.loaded) logs.load()
  })

  const visible = $derived.by(() => {
    const needle = query.trim().toLowerCase()
    const max = RANK[level]
    return logs.entries.filter(
      (e) =>
        RANK[e.level] <= max &&
        (!needle || e.message.toLowerCase().includes(needle) || e.target.toLowerCase().includes(needle)),
    )
  })

  // Stick to the bottom while following; scrolling up pauses following.
  $effect(() => {
    void visible
    if (!follow) return
    tick().then(() => viewport?.scrollTo({ top: viewport.scrollHeight }))
  })

  function onscroll() {
    if (!viewport) return
    const atBottom = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight < 24
    if (follow !== atBottom) follow = atBottom
  }

  async function clear() {
    const { confirmed } = await ui.confirm({
      title: t('logs.clearTitle'),
      body: t('logs.clearBody'),
      confirmLabel: t('logs.clear'),
      danger: true,
    })
    if (confirmed) logs.clear()
  }

  const levels = $derived(
    (['ERROR', 'WARN', 'INFO', 'DEBUG'] as const).map((value) => ({ value, label: t(`logs.levels.${value}`) })),
  )
</script>

<PageHeader title={t('logs.title')} count={t('logs.shown', { count: visible.length })}>
  {#snippet actions()}
    <div class="flex flex-wrap items-center gap-2">
      <Button icon={Trash2} onclick={clear}>{t('logs.clear')}</Button>
      <Button icon={Download} href={api.logs.fileUrl()}>{t('logs.download')}</Button>
    </div>
  {/snippet}
</PageHeader>

<div class="mb-3 flex flex-wrap items-center gap-x-5 gap-y-3">
  <div class="flex items-center gap-2.5">
    <span class="text-small text-fg-muted" aria-hidden="true">{t('logs.levelLabel')}</span>
    <Segmented bind:value={level} options={levels} label={t('logs.levelLabel')} />
  </div>
  <SearchInput bind:value={query} placeholder={t('logs.searchPlaceholder')} class="min-w-48 flex-1 sm:max-w-80" />
  <label class="flex items-center gap-2.5 text-small text-fg-muted">
    <Toggle bind:checked={follow} ariaLabel={t('logs.follow')} />
    {t('logs.follow')}
  </label>
</div>

<div
  bind:this={viewport}
  {onscroll}
  class="h-[calc(100dvh-17rem)] min-h-80 overflow-auto rounded-panel border border-line bg-sunken font-mono text-xs leading-relaxed"
  role="log"
  aria-live="off"
>
  {#if visible.length === 0}
    <p class="p-6 text-center font-sans text-sm text-fg-faint">{t('logs.empty')}</p>
  {:else}
    <ol class="py-2">
      {#each visible as entry (entry.id)}
        <li
          class="log-row grid grid-cols-[4.75rem_3.25rem_minmax(0,1fr)] gap-x-3 px-3 py-px hover:bg-panel md:grid-cols-[4.75rem_3.25rem_10rem_minmax(0,1fr)]"
        >
          <time class="text-fg-faint">{formatLogTime(entry.ts_ms)}</time>
          <span class="font-semibold {toneText[logTone[entry.level]]}">{entry.level}</span>
          <span class="hidden truncate text-fg-faint md:block" title={entry.target}>{entry.target}</span>
          <span class="break-words whitespace-pre-wrap text-fg">{entry.message}</span>
        </li>
      {/each}
    </ol>
  {/if}
</div>

<style>
  .log-row {
    content-visibility: auto;
    contain-intrinsic-size: auto 1.25rem;
  }
</style>
