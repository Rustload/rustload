<script lang="ts">
  import ArrowDown from '@lucide/svelte/icons/arrow-down'
  import ArrowUp from '@lucide/svelte/icons/arrow-up'
  import Plus from '@lucide/svelte/icons/plus'
  import RefreshCw from '@lucide/svelte/icons/refresh-cw'
  import Trash2 from '@lucide/svelte/icons/trash-2'
  import { api, type Account } from '$lib/api'
  import AddAccountDialog from '$lib/components/settings/AddAccountDialog.svelte'
  import ProviderMark from '$lib/components/settings/ProviderMark.svelte'
  import Badge from '$lib/components/ui/Badge.svelte'
  import Button from '$lib/components/ui/Button.svelte'
  import Menu from '$lib/components/ui/Menu.svelte'
  import Panel from '$lib/components/ui/Panel.svelte'
  import PanelEmpty from '$lib/components/ui/PanelEmpty.svelte'
  import Toggle from '$lib/components/ui/Toggle.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { accounts } from '$lib/stores/catalog.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { runWithToast } from '$lib/stores/toasts.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { formatBytes, formatDate, formatRelative } from '$lib/utils/format'

  const debrid = $derived(settings.draft!.debrid)
  let adding = $state(false)
  let checking = $state<number | null>(null)

  async function update(account: Account, action: () => Promise<Account>) {
    await runWithToast(async () => accounts.replace(await action()))
    if (checking === account.id) checking = null
  }

  function check(account: Account) {
    checking = account.id
    update(account, () => api.accounts.check(account.id))
  }

  async function remove(account: Account) {
    const { confirmed } = await ui.confirm({
      title: t('settings.accounts.deleteTitle', {
        provider: accounts.providerName(account.provider),
        label: account.label,
      }),
      body: t('settings.accounts.deleteBody'),
      confirmLabel: t('common.remove'),
      danger: true,
    })
    if (confirmed) runWithToast(() => accounts.remove(account.id))
  }
</script>

<Panel description={t('settings.accounts.intro')} flush>
  {#snippet actions()}
    <Button size="sm" icon={Plus} onclick={() => (adding = true)} disabled={accounts.providers.length === 0}>
      {t('settings.accounts.add')}
    </Button>
  {/snippet}

  {#if accounts.list.length === 0}
    <PanelEmpty text={t('settings.accounts.empty')} />
  {:else}
    <ul class="divide-y divide-line border-t border-line">
      {#each accounts.list as account, i (account.id)}
        {@const provider = accounts.providerName(account.provider)}
        <li class="flex items-start gap-3 py-3.5 pr-3 pl-5 {account.enabled ? '' : 'opacity-70'}">
          <ProviderMark name={provider} />
          <div class="min-w-0 flex-1">
            <p class="flex flex-wrap items-center gap-x-2.5 gap-y-1">
              <span class="font-medium">{account.label || provider}</span>
              {#if account.label && account.label !== provider}<span class="text-small text-fg-muted">{provider}</span
                >{/if}
              {#if account.premium}
                <Badge tone="ok" label={t('settings.accounts.premium')} />
              {:else}
                <Badge tone="warn" label={t('settings.accounts.free')} />
              {/if}
            </p>
            <p class="mt-1 flex flex-wrap gap-x-4 gap-y-0.5 num text-small text-fg-muted">
              {#if account.username}<span>{account.username}</span>{/if}
              {#if account.premium && account.premium_until}
                <span>{t('settings.accounts.until', { date: formatDate(account.premium_until) })}</span>
              {/if}
              {#if account.traffic_left !== null}
                <span>{t('settings.accounts.trafficLeft', { size: formatBytes(account.traffic_left) })}</span>
              {/if}
              <span class="font-mono text-xs leading-5 text-fg-faint">{account.api_key_masked}</span>
              <span class="text-fg-faint"
                >{t('settings.accounts.lastCheck', { when: formatRelative(account.last_check) })}</span
              >
            </p>
            {#if account.last_error}
              <p class="mt-1 text-small break-words text-bad">{account.last_error}</p>
            {/if}
          </div>
          <div class="flex items-center gap-1 pt-1">
            <Toggle
              checked={account.enabled}
              ariaLabel={t('settings.accounts.enabledLabel')}
              onchange={(enabled) => update(account, () => api.accounts.update(account.id, { enabled }))}
            />
            <Menu
              items={[
                {
                  label: t('settings.accounts.check'),
                  icon: RefreshCw,
                  disabled: checking === account.id,
                  onselect: () => check(account),
                },
                {
                  label: t('settings.accounts.moveUp'),
                  icon: ArrowUp,
                  disabled: i === 0,
                  onselect: () => runWithToast(() => accounts.move(i, -1)),
                },
                {
                  label: t('settings.accounts.moveDown'),
                  icon: ArrowDown,
                  disabled: i === accounts.list.length - 1,
                  onselect: () => runWithToast(() => accounts.move(i, 1)),
                },
                'separator',
                { label: t('common.remove'), icon: Trash2, danger: true, onselect: () => remove(account) },
              ]}
            />
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</Panel>

<Panel>
  <Toggle bind:checked={debrid.enabled} label={t('settings.accounts.debridEnabled')} />
  <Toggle bind:checked={debrid.delete_magnets} label={t('settings.accounts.deleteMagnets')} />
</Panel>

<AddAccountDialog bind:open={adding} />
