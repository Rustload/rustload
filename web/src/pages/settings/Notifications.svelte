<script lang="ts">
  import Pencil from '@lucide/svelte/icons/pencil'
  import Plus from '@lucide/svelte/icons/plus'
  import Send from '@lucide/svelte/icons/send'
  import Trash2 from '@lucide/svelte/icons/trash-2'
  import { api, type NotificationTarget } from '$lib/api'
  import NotificationDialog, { newTarget } from '$lib/components/settings/NotificationDialog.svelte'
  import ProviderMark from '$lib/components/settings/ProviderMark.svelte'
  import Button from '$lib/components/ui/Button.svelte'
  import Menu from '$lib/components/ui/Menu.svelte'
  import Panel from '$lib/components/ui/Panel.svelte'
  import PanelEmpty from '$lib/components/ui/PanelEmpty.svelte'
  import Toggle from '$lib/components/ui/Toggle.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { toasts } from '$lib/stores/toasts.svelte'
  import { ui } from '$lib/stores/ui.svelte'

  const notifications = $derived(settings.draft!.notifications)

  let editing = $state<NotificationTarget>(newTarget())
  let isNew = $state(true)
  let dialogOpen = $state(false)

  function open(target: NotificationTarget | null) {
    isNew = target === null
    editing = target ?? newTarget()
    dialogOpen = true
  }

  function save(target: NotificationTarget) {
    const index = notifications.targets.findIndex((x) => x.id === target.id)
    if (index === -1) notifications.targets.push(target)
    else notifications.targets[index] = target
  }

  async function remove(target: NotificationTarget) {
    const { confirmed } = await ui.confirm({
      title: t('settings.notifications.deleteTitle', { name: target.name }),
      confirmLabel: t('common.delete'),
      danger: true,
    })
    if (confirmed) notifications.targets = notifications.targets.filter((x) => x.id !== target.id)
  }

  async function test(target: NotificationTarget) {
    try {
      await api.notifications.test($state.snapshot(target))
      toasts.success(t('settings.notifications.testSent'))
    } catch (err) {
      toasts.error(err)
    }
  }
</script>

<Panel description={t('settings.notifications.intro')} flush>
  {#snippet actions()}
    <Button size="sm" icon={Plus} onclick={() => open(null)}>{t('settings.notifications.add')}</Button>
  {/snippet}

  {#if notifications.targets.length === 0}
    <PanelEmpty text={t('settings.notifications.empty')} />
  {:else}
    <ul class="divide-y divide-line border-t border-line">
      {#each notifications.targets as target (target.id)}
        <li class="flex items-center gap-3 py-3 pr-3 pl-5">
          <ProviderMark name={t(`settings.notifications.kinds.${target.kind}`)} />
          <div class="min-w-0 flex-1">
            <p class="truncate font-medium">{target.name}</p>
            <p class="truncate text-small text-fg-muted">
              {target.events.map((e) => t(`settings.notifications.eventNames.${e}`)).join(', ') || t('common.none')}
            </p>
          </div>
          <Toggle
            bind:checked={target.enabled}
            ariaLabel={t('settings.notifications.enabledLabel', { name: target.name })}
          />
          <Menu
            items={[
              { label: t('common.edit'), icon: Pencil, onselect: () => open(target) },
              { label: t('settings.notifications.test'), icon: Send, onselect: () => test(target) },
              'separator',
              { label: t('common.delete'), icon: Trash2, danger: true, onselect: () => remove(target) },
            ]}
          />
        </li>
      {/each}
    </ul>
  {/if}
</Panel>

<NotificationDialog bind:open={dialogOpen} target={editing} {isNew} onsave={save} />
