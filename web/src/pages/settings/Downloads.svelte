<script lang="ts">
  import Field from '$lib/components/ui/Field.svelte'
  import NumberInput from '$lib/components/ui/NumberInput.svelte'
  import Panel from '$lib/components/ui/Panel.svelte'
  import Select from '$lib/components/ui/Select.svelte'
  import TextInput from '$lib/components/ui/TextInput.svelte'
  import Toggle from '$lib/components/ui/Toggle.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { formatNumber, formatSpeed, kibToBytes } from '$lib/utils/format'

  const d = $derived(settings.draft!.downloads)

  const fileExistsOptions = $derived(
    (['rename', 'overwrite', 'skip'] as const).map((value) => ({
      value,
      label: t(`settings.downloads.fileExistsOptions.${value}`),
    })),
  )
</script>

<Panel title={t('settings.downloads.groupTransfers')}>
  <div class="grid gap-5 sm:grid-cols-2">
    <Field label={t('settings.downloads.parallel')} hint={t('settings.downloads.parallelHint')}>
      {#snippet children({ id, describedBy })}
        <NumberInput {id} {describedBy} bind:value={d.max_parallel} min={1} max={20} class="max-w-40" />
      {/snippet}
    </Field>
    <Field label={t('settings.downloads.connections')} hint={t('settings.downloads.connectionsHint')}>
      {#snippet children({ id, describedBy })}
        <NumberInput {id} {describedBy} bind:value={d.connections} min={1} max={32} class="max-w-40" />
      {/snippet}
    </Field>
    <Field label={t('settings.downloads.minSplit')} hint={t('settings.downloads.minSplitHint')}>
      {#snippet children({ id, describedBy })}
        <NumberInput
          {id}
          {describedBy}
          bind:value={d.min_split_size_mib}
          min={1}
          max={64}
          suffix={t('units.mib')}
          class="max-w-40"
        />
      {/snippet}
    </Field>
    <Field label={t('settings.downloads.timeout')}>
      {#snippet children({ id, describedBy })}
        <NumberInput
          {id}
          {describedBy}
          bind:value={d.timeout_secs}
          min={5}
          max={600}
          suffix={t('settings.downloads.secondsSuffix')}
          class="max-w-40"
        />
      {/snippet}
    </Field>
    <Field label={t('settings.downloads.retries')} hint={t('settings.downloads.retriesHint')}>
      {#snippet children({ id, describedBy })}
        <NumberInput {id} {describedBy} bind:value={d.max_retries} min={0} max={50} class="max-w-40" />
      {/snippet}
    </Field>
    <Field label={t('settings.downloads.retryDelay')}>
      {#snippet children({ id, describedBy })}
        <NumberInput
          {id}
          {describedBy}
          bind:value={d.retry_delay_secs}
          min={1}
          max={3600}
          suffix={t('settings.downloads.secondsSuffix')}
          class="max-w-40"
        />
      {/snippet}
    </Field>
  </div>
</Panel>

<Panel title={t('settings.downloads.groupSpeed')}>
  <Toggle bind:checked={d.speed_limit_enabled} label={t('settings.downloads.speedLimit')} />
  <Field
    label={t('settings.downloads.speedLimitValue')}
    hint={t('settings.downloads.speedLimitHint', {
      kib: formatNumber(d.speed_limit_kib),
      speed: formatSpeed(kibToBytes(d.speed_limit_kib), { trim: true }),
    })}
  >
    {#snippet children({ id, describedBy })}
      <NumberInput
        {id}
        {describedBy}
        bind:value={d.speed_limit_kib}
        min={0}
        step={512}
        suffix={t('settings.downloads.kibSuffix')}
        disabled={!d.speed_limit_enabled}
        class="max-w-56"
      />
    {/snippet}
  </Field>
</Panel>

<Panel title={t('settings.downloads.groupBehaviour')}>
  <Toggle
    bind:checked={d.auto_start}
    label={t('settings.downloads.autoStart')}
    hint={t('settings.downloads.autoStartHint')}
  />
  <Field label={t('settings.downloads.fileExists')}>
    {#snippet children({ id })}
      <Select {id} bind:value={d.file_exists} options={fileExistsOptions} class="max-w-sm" />
    {/snippet}
  </Field>
  <Field label={t('settings.downloads.userAgent')}>
    {#snippet children({ id })}
      <TextInput {id} bind:value={d.user_agent} mono spellcheck={false} autocomplete="off" />
    {/snippet}
  </Field>
</Panel>
