<script lang="ts">
  import CircleCheck from '@lucide/svelte/icons/circle-check'
  import Download from '@lucide/svelte/icons/download'
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert'
  import { onMount } from 'svelte'
  import { api, type MediaStatus } from '$lib/api'
  import MediaOptionsFields from '$lib/components/add/MediaOptionsFields.svelte'
  import Button from '$lib/components/ui/Button.svelte'
  import Field from '$lib/components/ui/Field.svelte'
  import Panel from '$lib/components/ui/Panel.svelte'
  import TextInput from '$lib/components/ui/TextInput.svelte'
  import Toggle from '$lib/components/ui/Toggle.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { toasts } from '$lib/stores/toasts.svelte'
  import { formatRelative } from '$lib/utils/format'

  const m = $derived(settings.draft!.media)

  let status = $state<MediaStatus | null>(null)

  const ytdlp = $derived(status?.ytdlp ?? null)
  const updateAvailable = $derived(
    !!ytdlp?.managed && !!status?.latest_version && status.latest_version !== ytdlp.version,
  )
  const installLabel = $derived(
    !ytdlp
      ? t('settings.media.install')
      : ytdlp.managed
        ? t('settings.media.update')
        : t('settings.media.installManaged'),
  )
  let installing = $state(false)
  let advanced = $state(false)

  async function refresh() {
    try {
      status = await api.media.status()
    } catch (err) {
      toasts.error(err)
    }
  }

  onMount(() => {
    refresh()
  })

  // An installation or an automatic update is running: check again until it's over.
  $effect(() => {
    if (!status?.updating || installing) return
    const timer = setTimeout(refresh, 3000)
    return () => clearTimeout(timer)
  })

  async function install() {
    installing = true
    try {
      status = await api.media.install()
      toasts.success(t('settings.media.installed', { version: status.ytdlp?.version ?? '' }))
    } catch (err) {
      toasts.error(err)
    } finally {
      installing = false
    }
  }
</script>

<Panel title={t('settings.media.tools')} description={t('settings.media.intro')}>
  <Toggle bind:checked={m.enabled} label={t('settings.media.enabled')} hint={t('settings.media.enabledHint')} />

  {#if status}
    <dl class="divide-y divide-line border-y border-line text-small">
      <div class="grid gap-1 py-2.5 sm:grid-cols-[8rem_minmax(0,1fr)_auto] sm:items-center sm:gap-4">
        <dt class="font-medium">yt-dlp</dt>
        <dd class="flex min-w-0 items-center gap-2">
          {#if ytdlp}
            <CircleCheck size={15} class="shrink-0 text-ok" aria-hidden="true" />
            <span class="num">{ytdlp.version ?? '?'}</span>
            {#if updateAvailable}
              <span class="shrink-0 text-small text-info">
                {t('settings.media.updateAvailable', { version: status.latest_version ?? '' })}
              </span>
            {/if}
            <span class="truncate font-mono text-xs text-fg-faint">{ytdlp.path}</span>
          {:else}
            <TriangleAlert size={15} class="shrink-0 text-warn" aria-hidden="true" />
            <span class="text-warn">{t('settings.media.missing')}</span>
          {/if}
        </dd>
        <dd>
          <Button size="sm" icon={Download} loading={installing || status.updating} onclick={install}>
            {installLabel}
          </Button>
        </dd>
      </div>
      <div class="grid gap-1 py-2.5 sm:grid-cols-[8rem_minmax(0,1fr)] sm:items-center sm:gap-4">
        <dt class="font-medium">ffmpeg</dt>
        <dd class="flex min-w-0 items-center gap-2">
          {#if status.ffmpeg}
            <CircleCheck size={15} class="shrink-0 text-ok" aria-hidden="true" />
            <span class="num">{status.ffmpeg.version ?? '?'}</span>
            <span class="truncate font-mono text-xs text-fg-faint">{status.ffmpeg.path}</span>
          {:else}
            <TriangleAlert size={15} class="shrink-0 text-warn" aria-hidden="true" />
            <span class="text-warn">{t('settings.media.noFfmpeg')}</span>
          {/if}
        </dd>
      </div>
    </dl>
    {#if ytdlp && !ytdlp.managed}
      <p class="text-small text-fg-muted">{t('settings.media.managedHint')}</p>
    {/if}
    {#if ytdlp?.managed}
      <Toggle
        bind:checked={m.auto_update}
        label={t('settings.media.autoUpdate')}
        hint={status.last_update_check
          ? t('settings.media.lastCheck', { when: formatRelative(status.last_update_check) })
          : undefined}
      />
    {/if}
  {/if}
</Panel>

<Panel title={t('settings.media.defaults')} description={t('settings.media.defaultsHint')}>
  <MediaOptionsFields bind:options={m.defaults} full />
</Panel>

<Panel title={t('settings.media.advanced')}>
  <Toggle
    bind:checked={m.generic_pages}
    label={t('settings.media.genericPages')}
    hint={t('settings.media.genericPagesHint')}
  />
  <Field label={t('settings.media.cookies')} hint={t('settings.media.cookiesHint')} optional>
    {#snippet children({ id, describedBy })}
      <TextInput
        {id}
        aria-describedby={describedBy}
        bind:value={m.cookies_file}
        mono
        spellcheck={false}
        placeholder="/config/cookies.txt"
      />
    {/snippet}
  </Field>
  {#if advanced}
    <Field label={t('settings.media.extraArgs')} hint={t('settings.media.extraArgsHint')} optional>
      {#snippet children({ id, describedBy })}
        <TextInput
          {id}
          aria-describedby={describedBy}
          bind:value={m.extra_args}
          mono
          spellcheck={false}
          placeholder="--geo-bypass"
        />
      {/snippet}
    </Field>
    <div class="grid gap-4 sm:grid-cols-2">
      <Field label={t('settings.media.ytdlpPath')} optional>
        {#snippet children({ id })}
          <TextInput {id} bind:value={m.ytdlp_path} mono spellcheck={false} placeholder={t('settings.media.auto')} />
        {/snippet}
      </Field>
      <Field label={t('settings.media.ffmpegPath')} optional>
        {#snippet children({ id })}
          <TextInput {id} bind:value={m.ffmpeg_path} mono spellcheck={false} placeholder={t('settings.media.auto')} />
        {/snippet}
      </Field>
    </div>
  {:else}
    <Button size="sm" variant="ghost" onclick={() => (advanced = true)}>{t('settings.media.showAdvanced')}</Button>
  {/if}
</Panel>
