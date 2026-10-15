<script lang="ts" module>
  import type { NotificationEvent, NotificationKind, NotificationTarget } from '$lib/api'

  export const NOTIFICATION_KINDS: NotificationKind[] = ['webhook', 'discord', 'telegram', 'ntfy', 'gotify']
  export const NOTIFICATION_EVENTS: NotificationEvent[] = [
    'package_finished',
    'package_failed',
    'extract_failed',
    'checksum_failed',
    'disk_low',
    'queue_finished',
  ]

  /** Stable client-side id. `crypto.randomUUID` needs a secure context, which plain-HTTP LAN setups lack. */
  function uid(): string {
    return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
  }

  export function newTarget(): NotificationTarget {
    return {
      id: uid(),
      name: '',
      kind: 'discord',
      enabled: true,
      url: '',
      token: '',
      chat_id: '',
      events: ['package_finished', 'package_failed', 'extract_failed'],
    }
  }
</script>

<script lang="ts">
  import Send from '@lucide/svelte/icons/send'
  import { untrack } from 'svelte'
  import { api } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { errorMessage, toasts } from '$lib/stores/toasts.svelte'
  import Alert from '../ui/Alert.svelte'
  import Button from '../ui/Button.svelte'
  import Checkbox from '../ui/Checkbox.svelte'
  import Dialog from '../ui/Dialog.svelte'
  import Field from '../ui/Field.svelte'
  import Select from '../ui/Select.svelte'
  import TextInput from '../ui/TextInput.svelte'

  type Props = {
    open: boolean
    /** Target to edit; a copy is edited and handed back through `onsave`. */
    target: NotificationTarget
    isNew: boolean
    onsave: (target: NotificationTarget) => void
  }

  let { open = $bindable(), target, isNew, onsave }: Props = $props()

  let draft = $state<NotificationTarget>(newTarget())
  let testing = $state(false)
  let testError = $state<string | null>(null)

  $effect(() => {
    if (!open) return
    untrack(() => {
      draft = JSON.parse(JSON.stringify(target))
      testError = null
    })
  })

  const kinds = $derived(NOTIFICATION_KINDS.map((k) => ({ value: k, label: t(`settings.notifications.kinds.${k}`) })))
  const needsUrl = $derived(draft.kind !== 'telegram')
  const showToken = $derived(draft.kind !== 'discord')
  const valid = $derived(
    draft.name.trim() !== '' &&
      (!needsUrl || /^https?:\/\/\S+$/.test(draft.url.trim())) &&
      (draft.kind !== 'telegram' || (draft.token.trim() !== '' && draft.chat_id.trim() !== '')) &&
      (draft.kind !== 'gotify' || draft.token.trim() !== ''),
  )

  function toggleEvent(event: NotificationEvent, on: boolean) {
    draft.events = on ? [...draft.events, event] : draft.events.filter((e) => e !== event)
  }

  async function test() {
    testing = true
    testError = null
    try {
      await api.notifications.test($state.snapshot(draft))
      toasts.success(t('settings.notifications.testSent'))
    } catch (err) {
      testError = errorMessage(err)
    } finally {
      testing = false
    }
  }

  function save(event: SubmitEvent) {
    event.preventDefault()
    if (!valid) return
    onsave({ ...$state.snapshot(draft), name: draft.name.trim(), url: draft.url.trim() })
    open = false
  }
</script>

<Dialog
  bind:open
  title={isNew ? t('settings.notifications.add') : t('settings.notifications.edit')}
  description={t('settings.notifications.unsavedHint')}
  initialFocus="input"
>
  <form id="notification-form" class="space-y-4" onsubmit={save}>
    <div class="grid gap-4 sm:grid-cols-2">
      <Field label={t('settings.notifications.name')}>
        {#snippet children({ id })}
          <TextInput {id} bind:value={draft.name} required />
        {/snippet}
      </Field>
      <Field label={t('settings.notifications.kind')}>
        {#snippet children({ id })}
          <Select {id} bind:value={draft.kind} options={kinds} />
        {/snippet}
      </Field>
    </div>

    <Field
      label={t(`settings.notifications.url.${draft.kind}`)}
      hint={t(`settings.notifications.urlHint.${draft.kind}`)}
      optional={!needsUrl}
    >
      {#snippet children({ id, describedBy })}
        <TextInput
          {id}
          bind:value={draft.url}
          type="url"
          mono
          spellcheck={false}
          autocomplete="off"
          aria-describedby={describedBy}
          required={needsUrl}
        />
      {/snippet}
    </Field>

    {#if showToken}
      <Field
        label={t(`settings.notifications.token.${draft.kind}`)}
        optional={draft.kind === 'webhook' || draft.kind === 'ntfy'}
      >
        {#snippet children({ id })}
          <TextInput {id} bind:value={draft.token} type="password" mono autocomplete="off" spellcheck={false} />
        {/snippet}
      </Field>
    {/if}

    {#if draft.kind === 'telegram'}
      <Field label={t('settings.notifications.chatId')}>
        {#snippet children({ id })}
          <TextInput {id} bind:value={draft.chat_id} mono autocomplete="off" spellcheck={false} required />
        {/snippet}
      </Field>
    {/if}

    <fieldset>
      <legend class="mb-2 font-medium">{t('settings.notifications.events')}</legend>
      <div class="grid gap-2 sm:grid-cols-2">
        {#each NOTIFICATION_EVENTS as event (event)}
          <Checkbox
            checked={draft.events.includes(event)}
            label={t(`settings.notifications.eventNames.${event}`)}
            onchange={(on) => toggleEvent(event, on)}
          />
        {/each}
      </div>
    </fieldset>

    {#if testError}
      <Alert class="text-small">{testError}</Alert>
    {/if}
  </form>

  {#snippet footer()}
    <Button icon={Send} loading={testing} disabled={!valid} onclick={test} class="mr-auto">
      {t('settings.notifications.test')}
    </Button>
    <Button onclick={() => (open = false)}>{t('common.cancel')}</Button>
    <Button type="submit" form="notification-form" variant="primary" disabled={!valid}>{t('common.apply')}</Button>
  {/snippet}
</Dialog>
