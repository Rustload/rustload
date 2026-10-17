<script lang="ts">
  import CircleCheck from '@lucide/svelte/icons/circle-check'
  import CircleX from '@lucide/svelte/icons/circle-x'
  import Download from '@lucide/svelte/icons/download'
  import RefreshCw from '@lucide/svelte/icons/refresh-cw'
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert'
  import Upload from '@lucide/svelte/icons/upload'
  import { onMount } from 'svelte'
  import { api, type DiagnosticCheck } from '$lib/api'
  import Alert from '$lib/components/ui/Alert.svelte'
  import Button from '$lib/components/ui/Button.svelte'
  import Loading from '$lib/components/ui/Loading.svelte'
  import Panel from '$lib/components/ui/Panel.svelte'
  import { hasMessage, t } from '$lib/i18n/index.svelte'
  import { auth } from '$lib/stores/auth.svelte'
  import { accounts, categories } from '$lib/stores/catalog.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { errorMessage, toasts } from '$lib/stores/toasts.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { checkTone, toneText } from '$lib/utils/status'

  let checks = $state<DiagnosticCheck[] | null>(null)
  let running = $state(false)
  let restoring = $state(false)
  let fileInput: HTMLInputElement | undefined = $state()

  const ICONS = { ok: CircleCheck, warn: TriangleAlert, error: CircleX }

  const summary = $derived.by(() => {
    if (!checks) return null
    const problems = checks.filter((c) => c.status !== 'ok').length
    return problems ? t('settings.system.problems', { count: problems }) : t('settings.system.allGood')
  })

  async function runChecks() {
    running = true
    try {
      checks = (await api.tools.diagnostics()).checks
    } catch (err) {
      toasts.error(err)
    } finally {
      running = false
    }
  }

  onMount(runChecks)

  /** Server titles are English; known checks get a translated title. */
  function title(check: DiagnosticCheck): string {
    const key = `settings.system.checks.${check.id}`
    return hasMessage(key) ? t(key) : check.title
  }

  async function restore(file: File) {
    let backup: unknown
    try {
      backup = JSON.parse(await file.text())
    } catch {
      toasts.push('error', t('settings.system.invalidFile'))
      return
    }
    const { confirmed, checked } = await ui.confirm({
      title: t('settings.system.restoreTitle'),
      body: t('settings.system.restoreBody'),
      checkbox: t('settings.system.restoreQueue'),
      confirmLabel: t('settings.system.restore'),
      danger: true,
    })
    if (!confirmed) return
    restoring = true
    try {
      const result = await api.tools.restore(backup, checked)
      await Promise.all([settings.load(), categories.load(), accounts.load(), queue.load()])
      toasts.success(t('settings.system.restored'))
      if (result.errors.length) {
        toasts.push(
          'warning',
          t('settings.system.restoreSkipped', { count: result.errors.length, first: result.errors[0] }),
        )
      }
    } catch (err) {
      toasts.push('error', errorMessage(err))
    } finally {
      restoring = false
    }
  }
</script>

<Panel
  title={t('settings.system.health')}
  description={summary ?? (auth.desktop ? t('settings.system.healthHintApp') : t('settings.system.healthHint'))}
>
  {#snippet actions()}
    <Button size="sm" icon={RefreshCw} loading={running} onclick={runChecks}>{t('settings.system.runAgain')}</Button>
  {/snippet}
  {#if checks}
    <ul class="divide-y divide-line border-y border-line">
      {#each checks as check (check.id)}
        {@const Icon = ICONS[check.status]}
        {@const tone = toneText[checkTone[check.status]]}
        <li class="flex gap-3 py-2.5">
          <Icon size={16} class="mt-0.5 shrink-0 {tone}" aria-label={t(`settings.system.status.${check.status}`)} />
          <div class="min-w-0 flex-1">
            <p class="font-medium">{title(check)}</p>
            <p class="text-small break-words text-fg-muted">{check.detail}</p>
            {#if check.hint}
              <p class="mt-0.5 text-small {tone}">{check.hint}</p>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {:else}
    <Loading compact />
  {/if}
</Panel>

<Panel title={t('settings.system.backup')} description={t('settings.system.backupHint')}>
  <Alert tone="warn" icon={TriangleAlert} class="text-small">{t('settings.system.backupSecrets')}</Alert>
  <div class="flex flex-wrap gap-2">
    <Button icon={Download} href={api.tools.backupUrl()} download>{t('settings.system.download')}</Button>
    <Button icon={Upload} loading={restoring} onclick={() => fileInput?.click()}>
      {t('settings.system.restoreFrom')}
    </Button>
    <input
      bind:this={fileInput}
      type="file"
      accept="application/json,.json"
      class="hidden"
      onchange={(e) => {
        const file = e.currentTarget.files?.[0]
        e.currentTarget.value = ''
        if (file) restore(file)
      }}
    />
  </div>
</Panel>
