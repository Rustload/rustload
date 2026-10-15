<script lang="ts">
  import Bug from '@lucide/svelte/icons/bug'
  import Code from '@lucide/svelte/icons/code'
  import Logo from '$lib/components/layout/Logo.svelte'
  import Button from '$lib/components/ui/Button.svelte'
  import Panel from '$lib/components/ui/Panel.svelte'
  import { ISSUES_URL, PROJECT_URL } from '$lib/config'
  import { t } from '$lib/i18n/index.svelte'
  import { system } from '$lib/stores/system.svelte'
  import { formatDuration } from '$lib/utils/format'

  const info = $derived(system.info)
  const rows = $derived(
    info
      ? [
          [t('settings.about.version'), info.version, true],
          [t('settings.about.root'), info.download_root, true],
          [t('settings.about.extractor'), info.extractor ?? t('settings.about.extractorNative'), !!info.extractor],
          [t('settings.about.uptime'), formatDuration(info.uptime_secs), false],
        ]
      : [],
  )
</script>

<Panel>
  <div class="flex items-center gap-4">
    <Logo size={48} tile />
    <div>
      <p class="text-xl font-semibold heading">Rustload</p>
      <p class="text-fg-muted">{t('settings.about.tagline')}</p>
    </div>
  </div>

  <dl class="divide-y divide-line border-y border-line">
    {#each rows as [label, value, mono] (label)}
      <div class="grid gap-1 py-2.5 sm:grid-cols-[12rem_minmax(0,1fr)] sm:gap-4">
        <dt class="text-fg-muted">{label}</dt>
        <dd class="break-all {mono ? 'font-mono text-small' : 'num'}">{value}</dd>
      </div>
    {/each}
  </dl>

  <div class="flex flex-wrap gap-2">
    <Button icon={Code} href={PROJECT_URL} external>{t('settings.about.source')}</Button>
    <Button icon={Bug} href={ISSUES_URL} external>{t('settings.about.issues')}</Button>
  </div>
  <p class="text-small text-fg-faint">{t('settings.about.license')}</p>
</Panel>
