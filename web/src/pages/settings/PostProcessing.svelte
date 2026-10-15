<script lang="ts">
  import Info from '@lucide/svelte/icons/info'
  import Field from '$lib/components/ui/Field.svelte'
  import LinesInput from '$lib/components/ui/LinesInput.svelte'
  import Panel from '$lib/components/ui/Panel.svelte'
  import Toggle from '$lib/components/ui/Toggle.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { system } from '$lib/stores/system.svelte'

  const p = $derived(settings.draft!.postprocess)
  const extractor = $derived(system.info?.extractor ?? null)
</script>

<Panel>
  <Toggle
    bind:checked={p.extract}
    label={t('settings.postprocess.extract')}
    hint={t('settings.postprocess.extractHint')}
  />
  <Toggle bind:checked={p.delete_archives} label={t('settings.postprocess.deleteArchives')} disabled={!p.extract} />
  <Toggle
    bind:checked={p.verify_checksums}
    label={t('settings.postprocess.verify')}
    hint={t('settings.postprocess.verifyHint')}
  />
  {#if system.info}
    <p class="flex gap-2 text-small {extractor ? 'text-fg-muted' : 'text-warn'}">
      <Info size={15} class="mt-0.5 shrink-0" aria-hidden="true" />
      {extractor
        ? t('settings.postprocess.extractorFound', { tool: extractor })
        : t('settings.postprocess.extractorMissing')}
    </p>
  {/if}
</Panel>

<Panel>
  <Field label={t('settings.postprocess.passwords')} hint={t('settings.postprocess.passwordsHint')}>
    {#snippet children({ id, describedBy })}
      <LinesInput {id} {describedBy} bind:values={p.passwords} rows={5} />
    {/snippet}
  </Field>
</Panel>
