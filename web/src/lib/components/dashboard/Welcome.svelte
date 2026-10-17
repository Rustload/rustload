<script lang="ts">
  import Circle from '@lucide/svelte/icons/circle'
  import CircleCheck from '@lucide/svelte/icons/circle-check'
  import X from '@lucide/svelte/icons/x'
  import { t } from '$lib/i18n/index.svelte'
  import { auth } from '$lib/stores/auth.svelte'
  import { router } from '$lib/router.svelte'
  import { accounts } from '$lib/stores/catalog.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { readStored, writeStored } from '$lib/utils/storage'
  import Button from '../ui/Button.svelte'
  import IconButton from '../ui/IconButton.svelte'
  import Panel from '../ui/Panel.svelte'

  /** Setup checklist shown on a fresh install until it's done or dismissed. */
  let { hasHistory }: { hasHistory: boolean } = $props()

  const DISMISSED = 'rustload.welcome.dismissed'
  const BROWSER = 'rustload.welcome.browser'

  let dismissed = $state(readStored(DISMISSED) === '1')
  let browserDone = $state(readStored(BROWSER) === '1')

  const steps = $derived([
    {
      id: 'account',
      done: accounts.list.length > 0,
      title: t('welcome.account'),
      body: t('welcome.accountBody'),
      action: t('welcome.accountAction'),
      run: () => router.go('/settings/accounts'),
    },
    {
      id: 'browser',
      done: browserDone,
      title: t('welcome.browser'),
      body: auth.desktop ? t('welcome.browserBodyApp') : t('welcome.browserBody'),
      action: t('welcome.browserAction'),
      run: () => {
        writeStored(BROWSER, '1')
        browserDone = true
        router.go('/settings/integrations')
      },
    },
    {
      id: 'first',
      done: queue.packages.length > 0 || hasHistory,
      title: t('welcome.first'),
      body: t('welcome.firstBody'),
      action: t('welcome.firstAction'),
      run: () => ui.openAdd(),
    },
  ])

  const visible = $derived(!dismissed && steps.some((s) => !s.done))

  function dismiss() {
    writeStored(DISMISSED, '1')
    dismissed = true
  }
</script>

{#if visible}
  <Panel title={t('welcome.title')} description={t('welcome.intro')} headingId="welcome-title" flush>
    {#snippet actions()}
      <IconButton icon={X} label={t('welcome.dismiss')} size="sm" class="-mr-1.5" onclick={dismiss} />
    {/snippet}
    <ul class="divide-y divide-line border-t border-line">
      {#each steps as step (step.id)}
        <li class="flex flex-wrap items-center gap-x-4 gap-y-2 px-5 py-3">
          {#if step.done}
            <CircleCheck size={18} class="shrink-0 text-ok" aria-label={t('welcome.done')} />
          {:else}
            <Circle size={18} class="shrink-0 text-fg-faint" aria-hidden="true" />
          {/if}
          <div class="min-w-0 flex-1 basis-60">
            <p class="font-medium {step.done ? 'text-fg-muted line-through decoration-fg-faint' : ''}">{step.title}</p>
            <p class="text-small text-fg-muted">{step.body}</p>
          </div>
          {#if !step.done}
            <Button size="sm" variant="secondary" onclick={step.run}>
              {step.action}
            </Button>
          {/if}
        </li>
      {/each}
    </ul>
  </Panel>
{/if}
