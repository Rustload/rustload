<script lang="ts">
  import LogOut from '@lucide/svelte/icons/log-out'
  import { ApiError, api } from '$lib/api'
  import Alert from '$lib/components/ui/Alert.svelte'
  import Button from '$lib/components/ui/Button.svelte'
  import Field from '$lib/components/ui/Field.svelte'
  import Panel from '$lib/components/ui/Panel.svelte'
  import TextInput from '$lib/components/ui/TextInput.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { auth } from '$lib/stores/auth.svelte'
  import { errorMessage, toasts } from '$lib/stores/toasts.svelte'
  import { passwordErrors } from '$lib/utils/password'

  let current = $state('')
  let next = $state('')
  let confirm = $state('')
  let busy = $state(false)
  let error = $state<string | null>(null)

  const rules = $derived(passwordErrors(next, confirm))
  // The desktop app's account starts with a random password nobody knows.
  const canSubmit = $derived((auth.desktop || current !== '') && rules.ok)

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    if (!canSubmit) return
    busy = true
    error = null
    try {
      await api.auth.changePassword(current, next)
      current = next = confirm = ''
      toasts.success(t('settings.security.changed'))
    } catch (err) {
      error =
        err instanceof ApiError && err.code === 'INVALID_CREDENTIALS'
          ? t('settings.security.wrongCurrent')
          : errorMessage(err)
    } finally {
      busy = false
    }
  }
</script>

{#if !auth.enabled}
  <Panel>
    <p class="text-fg-muted">{t('settings.security.authDisabled')}</p>
  </Panel>
{:else}
  <Panel
    title={auth.desktop ? t('settings.security.desktopPassword') : t('settings.security.changePassword')}
    description={auth.desktop
      ? t('settings.security.desktopPasswordHint', { address: window.location.origin, user: auth.username ?? '' })
      : undefined}
  >
    <form class="max-w-sm space-y-4" onsubmit={submit} novalidate>
      <input type="text" name="username" value={auth.username ?? ''} autocomplete="username" hidden readonly />
      {#if !auth.desktop}
        <Field label={t('settings.security.current')}>
          {#snippet children({ id })}
            <TextInput {id} type="password" bind:value={current} autocomplete="current-password" required />
          {/snippet}
        </Field>
      {/if}
      <Field label={t('settings.security.next')} error={rules.next}>
        {#snippet children({ id, describedBy, invalid })}
          <TextInput
            {id}
            type="password"
            bind:value={next}
            {invalid}
            aria-describedby={describedBy}
            autocomplete="new-password"
            required
          />
        {/snippet}
      </Field>
      <Field label={t('settings.security.confirm')} error={rules.confirm}>
        {#snippet children({ id, describedBy, invalid })}
          <TextInput
            {id}
            type="password"
            bind:value={confirm}
            {invalid}
            aria-describedby={describedBy}
            autocomplete="new-password"
            required
          />
        {/snippet}
      </Field>
      {#if error}
        <Alert>{error}</Alert>
      {/if}
      <Button type="submit" variant="primary" loading={busy} disabled={!canSubmit}>
        {t('settings.security.submit')}
      </Button>
    </form>
  </Panel>

  {#if !auth.desktop}
    <Panel title={t('settings.security.session')} description={t('settings.security.signOutHint')}>
      <Button icon={LogOut} onclick={() => auth.logout()}>{t('shell.signOut')}</Button>
    </Panel>
  {/if}
{/if}
