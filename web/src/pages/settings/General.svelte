<script lang="ts">
  import Monitor from '@lucide/svelte/icons/monitor'
  import Moon from '@lucide/svelte/icons/moon'
  import Sun from '@lucide/svelte/icons/sun'
  import type { Settings } from '$lib/api'
  import Field from '$lib/components/ui/Field.svelte'
  import Panel from '$lib/components/ui/Panel.svelte'
  import Segmented from '$lib/components/ui/Segmented.svelte'
  import Select from '$lib/components/ui/Select.svelte'
  import Toggle from '$lib/components/ui/Toggle.svelte'
  import { LOCALES, i18n, t } from '$lib/i18n/index.svelte'
  import { appWindow } from '$lib/utils/app'
  import { auth } from '$lib/stores/auth.svelte'
  import { desktop } from '$lib/stores/desktop.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { theme } from '$lib/stores/theme.svelte'

  /** Bumped after each answer so the switch is redrawn from the real state. */
  let answered = $state(0)

  type General = Settings['general']

  // Language and theme apply (and save) immediately instead of going through the draft.
  function setLanguage(value: General['language']) {
    i18n.set(value)
    settings.patch((s) => (s.general.language = value))
  }

  const themes = $derived([
    { value: 'system' as const, label: t('settings.general.themes.system'), icon: Monitor },
    { value: 'light' as const, label: t('settings.general.themes.light'), icon: Sun },
    { value: 'dark' as const, label: t('settings.general.themes.dark'), icon: Moon },
  ])
</script>

<Panel>
  <Field label={t('settings.general.language')}>
    {#snippet children({ id })}
      <Select
        {id}
        value={i18n.locale}
        options={LOCALES.map((l) => ({ value: l.id, label: l.name }))}
        onchange={setLanguage}
        class="max-w-64"
      />
    {/snippet}
  </Field>
  <div>
    <p class="mb-1.5 font-medium">{t('settings.general.theme')}</p>
    <Segmented
      value={theme.preference}
      options={themes}
      label={t('settings.general.theme')}
      onchange={(value) => settings.setTheme(value)}
    />
  </div>
</Panel>

<!-- The desktop app shows system notifications itself. -->
{#if !auth.desktop && !appWindow}
  <Panel>
    <!-- Recreated when the browser answers, in case the permission was refused. -->
    {#key `${desktop.enabled}:${desktop.permission}:${answered}`}
      <Toggle
        checked={desktop.enabled}
        label={t('settings.general.desktop')}
        hint={!desktop.supported
          ? t('settings.general.desktopUnsupported')
          : desktop.permission === 'denied'
            ? t('settings.general.desktopBlocked')
            : t('settings.general.desktopHint')}
        disabled={!desktop.supported || desktop.permission === 'denied'}
        onchange={async (on) => {
          if (on) await desktop.enable()
          else desktop.disable()
          answered++
        }}
      />
    {/key}
  </Panel>
{/if}
