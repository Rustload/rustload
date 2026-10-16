<script lang="ts">
  import CircleCheck from '@lucide/svelte/icons/circle-check'
  import Download from '@lucide/svelte/icons/download'
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert'
  import { api } from '$lib/api'
  import Button from '$lib/components/ui/Button.svelte'
  import Field from '$lib/components/ui/Field.svelte'
  import LinesInput from '$lib/components/ui/LinesInput.svelte'
  import Panel from '$lib/components/ui/Panel.svelte'
  import Toggle from '$lib/components/ui/Toggle.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { system } from '$lib/stores/system.svelte'
  import { toasts } from '$lib/stores/toasts.svelte'

  const p = $derived(settings.draft!.postprocess)
  const extractor = $derived(system.info?.extractor ?? null)
  let installing = $state(false)

  async function install() {
    installing = true
    try {
      system.info = await api.system.installExtractor()
      toasts.success(t('settings.postprocess.extractorInstalled'))
    } catch (err) {
      toasts.error(err)
    } finally {
      installing = false
    }
  }
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
    <dl class="divide-y divide-line border-y border-line text-small">
      <div class="grid gap-1 py-2.5 sm:grid-cols-[8rem_minmax(0,1fr)_auto] sm:items-center sm:gap-4">
        <dt class="font-medium">7-Zip</dt>
        <dd class="flex min-w-0 items-center gap-2">
          {#if extractor}
            <CircleCheck size={15} class="shrink-0 text-ok" aria-hidden="true" />
            <span>{t('settings.postprocess.extractorFound', { tool: extractor })}</span>
          {:else}
            <TriangleAlert size={15} class="shrink-0 text-warn" aria-hidden="true" />
            <span class="text-warn">{t('settings.postprocess.extractorMissing')}</span>
          {/if}
        </dd>
        {#if !extractor && system.info.extractor_installable}
          <dd>
            <Button
              size="sm"
              icon={Download}
              loading={installing || system.info.installing_extractor}
              onclick={install}
            >
              {t('settings.media.install')}
            </Button>
          </dd>
        {/if}
      </div>
    </dl>
    {#if !extractor}
      <p class="text-small text-fg-muted">
        {system.info.extractor_installable
          ? t('settings.postprocess.extractorInstallHint')
          : t('settings.postprocess.extractorSystemHint')}
      </p>
    {/if}
  {/if}
</Panel>

<Panel>
  <Field label={t('settings.postprocess.passwords')} hint={t('settings.postprocess.passwordsHint')}>
    {#snippet children({ id, describedBy })}
      <LinesInput {id} {describedBy} bind:values={p.passwords} rows={5} />
    {/snippet}
  </Field>
</Panel>
