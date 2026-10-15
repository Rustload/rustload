<script lang="ts">
  import type { MediaOptions, MediaQuality } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import Field from '../ui/Field.svelte'
  import Segmented from '../ui/Segmented.svelte'
  import Select from '../ui/Select.svelte'
  import TextInput from '../ui/TextInput.svelte'
  import Toggle from '../ui/Toggle.svelte'

  /** Quality and extras for media sites (yt-dlp). `full` adds the embedding options of the settings. */
  let { options = $bindable(), full = false }: { options: MediaOptions; full?: boolean } = $props()

  const QUALITIES: MediaQuality[] = ['best', '2160', '1440', '1080', '720', '480', '360', 'audio']

  const qualityOptions = $derived(
    QUALITIES.map((value) => ({
      value,
      label:
        value === 'best' || value === 'audio'
          ? t(`media.quality.${value}`)
          : t('media.quality.height', { height: value }),
    })),
  )
  const audioOptions = $derived(
    (['best', 'm4a', 'mp3', 'opus', 'flac'] as const).map((value) => ({
      value,
      label: value === 'best' ? t('media.audioBest') : value.toUpperCase(),
    })),
  )
</script>

<div class="space-y-4">
  <Field label={t('media.qualityLabel')}>
    {#snippet children({ id })}
      <Select {id} bind:value={options.quality} options={qualityOptions} />
    {/snippet}
  </Field>

  {#if options.quality === 'audio'}
    <Field label={t('media.audioFormat')}>
      {#snippet children({ id })}
        <Select {id} bind:value={options.audio_format} options={audioOptions} />
      {/snippet}
    </Field>
  {:else}
    <div>
      <p class="mb-1.5 font-medium">{t('media.container')}</p>
      <Segmented
        label={t('media.container')}
        bind:value={options.container}
        options={[
          { value: 'auto', label: t('media.containerAuto') },
          { value: 'mp4', label: 'MP4' },
          { value: 'mkv', label: 'MKV' },
        ]}
      />
    </div>
  {/if}

  <div class="space-y-3.5 rounded-control border border-line px-4 py-3.5">
    <Toggle bind:checked={options.subtitles} label={t('media.subtitles')} />
    {#if options.subtitles}
      <Field label={t('media.subtitleLangs')} hint={t('media.subtitleLangsHint')}>
        {#snippet children({ id, describedBy })}
          <TextInput {id} aria-describedby={describedBy} bind:value={options.subtitle_langs} spellcheck={false} />
        {/snippet}
      </Field>
    {/if}
    {#if full}
      <Toggle bind:checked={options.embed_metadata} label={t('media.embedMetadata')} />
      <Toggle bind:checked={options.embed_thumbnail} label={t('media.embedThumbnail')} />
    {/if}
  </div>

  <div>
    <p class="mb-1.5 font-medium">{t('media.sponsorblock')}</p>
    <Segmented
      label={t('media.sponsorblock')}
      bind:value={options.sponsorblock}
      options={[
        { value: 'off', label: t('media.sponsorOff') },
        { value: 'mark', label: t('media.sponsorMark') },
        { value: 'remove', label: t('media.sponsorRemove') },
      ]}
    />
    <p class="mt-1.5 text-small text-fg-muted">{t('media.sponsorblockHint')}</p>
  </div>
</div>
