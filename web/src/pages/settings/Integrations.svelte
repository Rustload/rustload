<script lang="ts">
  import Copy from '@lucide/svelte/icons/copy'
  import Eye from '@lucide/svelte/icons/eye'
  import EyeOff from '@lucide/svelte/icons/eye-off'
  import RefreshCw from '@lucide/svelte/icons/refresh-cw'
  import Send from '@lucide/svelte/icons/send'
  import { api } from '$lib/api'
  import BrowserPanel from '$lib/components/settings/BrowserPanel.svelte'
  import Button from '$lib/components/ui/Button.svelte'
  import Field from '$lib/components/ui/Field.svelte'
  import IconButton from '$lib/components/ui/IconButton.svelte'
  import Panel from '$lib/components/ui/Panel.svelte'
  import Select from '$lib/components/ui/Select.svelte'
  import TagInput from '$lib/components/ui/TagInput.svelte'
  import TextInput from '$lib/components/ui/TextInput.svelte'
  import Toggle from '$lib/components/ui/Toggle.svelte'
  import { t, type MessageKey } from '$lib/i18n/index.svelte'
  import { isWindows } from '$lib/utils/app'
  import { auth } from '$lib/stores/auth.svelte'
  import { categories } from '$lib/stores/catalog.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { errorMessage, toasts } from '$lib/stores/toasts.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { copyText } from '$lib/utils/clipboard'

  const s = $derived(settings.draft!)
  const apiKey = $derived(settings.current?.integrations.api_key ?? '')

  let revealed = $state(false)
  let regenerating = $state(false)
  let testingBot = $state(false)
  let botResult = $state<{ ok: boolean; text: string } | null>(null)

  const metricsUrl = `${window.location.origin}${window.location.pathname}metrics`
  const scrapeConfig = $derived(
    [
      'scrape_configs:',
      '  - job_name: rustload',
      ...(window.location.protocol === 'https:' ? ['    scheme: https'] : []),
      `    metrics_path: ${new URL(metricsUrl).pathname}`,
      '    authorization:',
      `      credentials: ${revealed ? apiKey : '<api key>'}`,
      '    static_configs:',
      `      - targets: ['${window.location.host}']`,
    ].join('\n'),
  )

  async function testBot() {
    testingBot = true
    botResult = null
    try {
      const { username } = await api.tools.testTelegram(s.telegram_bot.token.trim())
      botResult = { ok: true, text: t('settings.telegram.testOk', { name: `@${username}` }) }
    } catch (err) {
      botResult = { ok: false, text: errorMessage(err) }
    } finally {
      testingBot = false
    }
  }

  const ENV_VARS: [string, MessageKey][] = [
    ['RUSTLOAD_EVENT', 'settings.integrations.envVars.event'],
    ['RUSTLOAD_PACKAGE_ID', 'settings.integrations.envVars.id'],
    ['RUSTLOAD_PACKAGE_NAME', 'settings.integrations.envVars.name'],
    ['RUSTLOAD_PACKAGE_DIR', 'settings.integrations.envVars.dir'],
    ['RUSTLOAD_PACKAGE_STATUS', 'settings.integrations.envVars.status'],
    ['RUSTLOAD_PACKAGE_SIZE', 'settings.integrations.envVars.size'],
    ['RUSTLOAD_FILES', 'settings.integrations.envVars.files'],
  ]

  const categoryOptions = $derived([
    { value: null as number | null, label: t('add.automatic') },
    ...categories.list.map((c) => ({ value: c.id as number | null, label: c.name })),
  ])

  async function copy() {
    if (await copyText(apiKey)) toasts.success(t('common.copied'))
  }

  async function regenerate() {
    const { confirmed } = await ui.confirm({
      title: t('settings.integrations.regenerateTitle'),
      body: t('settings.integrations.regenerateBody'),
      confirmLabel: t('settings.integrations.regenerate'),
      danger: true,
    })
    if (!confirmed) return
    regenerating = true
    try {
      const { api_key } = await api.settings.regenerateApiKey()
      settings.setApiKey(api_key)
      revealed = true
      toasts.success(t('settings.integrations.regenerated'))
    } catch (err) {
      toasts.error(err)
    } finally {
      regenerating = false
    }
  }

  const hookPlaceholder = $derived(
    !auth.desktop
      ? t('settings.integrations.hookPlaceholder')
      : isWindows
        ? 'C:\\Scripts\\on-finished.cmd'
        : '~/scripts/on-finished.sh',
  )
</script>

<Panel title={t('settings.integrations.apiKey')} description={t('settings.integrations.apiKeyHint')}>
  <div class="flex flex-wrap items-center gap-2">
    <code
      class="min-w-0 flex-1 truncate rounded-control border border-line bg-sunken px-3 py-2 font-mono text-small select-all"
    >
      {revealed ? apiKey : '•'.repeat(Math.min(32, apiKey.length || 32))}
    </code>
    <IconButton
      icon={revealed ? EyeOff : Eye}
      label={revealed ? t('common.hide') : t('common.show')}
      variant="secondary"
      onclick={() => (revealed = !revealed)}
    />
    <IconButton icon={Copy} label={t('common.copy')} variant="secondary" onclick={copy} disabled={!apiKey} />
  </div>
  <Button icon={RefreshCw} loading={regenerating} onclick={regenerate}>{t('settings.integrations.regenerate')}</Button>
</Panel>

<BrowserPanel />

<Panel title={t('settings.telegram.title')} description={t('settings.telegram.intro')}>
  <Toggle bind:checked={s.telegram_bot.enabled} label={t('settings.telegram.enabled')} />
  {#if s.telegram_bot.enabled}
    <ol class="list-decimal space-y-1 pl-5 text-small text-fg-muted marker:num">
      <li>{t('settings.telegram.step1')}</li>
      <li>{t('settings.telegram.step2')}</li>
      <li>{t('settings.telegram.step3')}</li>
    </ol>
    <Field label={t('settings.telegram.token')}>
      {#snippet children({ id })}
        <div class="flex gap-2">
          <TextInput
            {id}
            type="password"
            bind:value={s.telegram_bot.token}
            mono
            autocomplete="off"
            spellcheck={false}
            placeholder="123456789:AA…"
            class="flex-1"
          />
          <Button icon={Send} loading={testingBot} disabled={!s.telegram_bot.token.trim()} onclick={testBot}>
            {t('settings.telegram.test')}
          </Button>
        </div>
      {/snippet}
    </Field>
    {#if botResult}
      <p role="status" class="text-small {botResult.ok ? 'text-ok' : 'text-bad'}">{botResult.text}</p>
    {/if}
    <Field label={t('settings.telegram.chats')} hint={t('settings.telegram.chatsHint')}>
      {#snippet children({ id, describedBy })}
        <TagInput {id} {describedBy} bind:values={s.telegram_bot.allowed_chats} placeholder="123456789" />
      {/snippet}
    </Field>
    <p class="text-small text-fg-muted">{t('settings.telegram.commands')}</p>
  {/if}
</Panel>

<Panel title={t('settings.integrations.watch')} description={t('settings.integrations.watchHint')}>
  <Toggle bind:checked={s.watch.enabled} label={t('settings.integrations.watchEnabled')} />
  {#if s.watch.enabled}
    <p class="text-small text-fg-muted">{t('settings.integrations.watchTxtHint')}</p>
    <div class="grid gap-4 sm:grid-cols-2">
      <Field label={t('settings.integrations.watchDir')}>
        {#snippet children({ id })}
          <TextInput {id} bind:value={s.watch.dir} mono spellcheck={false} autocomplete="off" />
        {/snippet}
      </Field>
      <Field label={t('settings.integrations.watchCategory')}>
        {#snippet children({ id })}
          <Select {id} bind:value={s.watch.category_id} options={categoryOptions} />
        {/snippet}
      </Field>
    </div>
  {/if}
</Panel>

<Panel
  title={t('settings.integrations.clicknload')}
  description={auth.desktop ? t('settings.integrations.clicknloadHintApp') : t('settings.integrations.clicknloadHint')}
>
  <Toggle bind:checked={s.clicknload.enabled} label={t('settings.integrations.clicknloadEnabled')} />
  <Toggle
    bind:checked={s.clicknload.paused}
    label={t('settings.integrations.clicknloadPaused')}
    disabled={!s.clicknload.enabled}
  />
</Panel>

<Panel title={t('settings.integrations.hooks')}>
  <Field label={t('settings.integrations.hookFinished')}>
    {#snippet children({ id })}
      <TextInput
        {id}
        bind:value={s.hooks.package_finished}
        mono
        spellcheck={false}
        autocomplete="off"
        placeholder={hookPlaceholder}
      />
    {/snippet}
  </Field>
  <Field label={t('settings.integrations.hookFailed')}>
    {#snippet children({ id })}
      <TextInput {id} bind:value={s.hooks.package_failed} mono spellcheck={false} autocomplete="off" />
    {/snippet}
  </Field>
  <div>
    <p class="text-small text-fg-muted">
      {auth.desktop
        ? t('settings.integrations.hooksHintApp', { shell: isWindows ? 'cmd' : 'sh' })
        : t('settings.integrations.hooksHint')}
    </p>
    <dl class="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-small">
      {#each ENV_VARS as [name, description] (name)}
        <dt class="font-mono text-xs leading-5">{name}</dt>
        <dd class="text-fg-muted">{t(description)}</dd>
      {/each}
    </dl>
  </div>
</Panel>

<!-- Prometheus scrapes servers; the app only answers on its own computer. -->
{#if !auth.desktop}
  <Panel title={t('settings.metrics.title')} description={t('settings.metrics.intro')}>
    <Toggle bind:checked={s.integrations.metrics} label={t('settings.metrics.enabled')} />
    {#if s.integrations.metrics}
      <p class="text-small text-fg-muted">{t('settings.metrics.auth')}</p>
      <pre
        class="overflow-x-auto rounded-control border border-line bg-sunken px-3 py-2.5 font-mono text-xs leading-relaxed">{scrapeConfig}</pre>
    {/if}
  </Panel>
{/if}
