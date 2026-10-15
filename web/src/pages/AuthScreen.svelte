<script lang="ts">
  import { ApiError } from '$lib/api'
  import FillStrip from '$lib/components/layout/FillStrip.svelte'
  import Logo from '$lib/components/layout/Logo.svelte'
  import Alert from '$lib/components/ui/Alert.svelte'
  import Button from '$lib/components/ui/Button.svelte'
  import Checkbox from '$lib/components/ui/Checkbox.svelte'
  import Field from '$lib/components/ui/Field.svelte'
  import TextInput from '$lib/components/ui/TextInput.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { auth } from '$lib/stores/auth.svelte'
  import { errorMessage } from '$lib/stores/toasts.svelte'
  import { passwordErrors } from '$lib/utils/password'

  let { mode }: { mode: 'setup' | 'login' } = $props()

  let username = $state('')
  let password = $state('')
  let confirm = $state('')
  let remember = $state(true)
  let busy = $state(false)
  let error = $state<string | null>(null)
  let usernameInput: HTMLInputElement | undefined = $state()

  // Signing in takes the password as it is; the rules only apply to a new one.
  const rules = $derived(mode === 'setup' ? passwordErrors(password, confirm) : null)
  const canSubmit = $derived(username.trim() !== '' && password !== '' && (rules?.ok ?? true))

  $effect(() => {
    usernameInput?.focus()
  })

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    if (!canSubmit) return
    busy = true
    error = null
    try {
      if (mode === 'setup') await auth.setup(username.trim(), password)
      else await auth.login(username.trim(), password, remember)
    } catch (err) {
      if (err instanceof ApiError && err.code === 'INVALID_CREDENTIALS') error = t('auth.invalid')
      else if (err instanceof ApiError && err.code === 'ALREADY_SETUP') {
        error = t('auth.alreadySetup')
        await auth.load()
      } else error = errorMessage(err)
      password = ''
    } finally {
      busy = false
    }
  }
</script>

<main class="grid min-h-dvh place-items-center px-5 py-10">
  <div class="w-full max-w-sm">
    <Logo size={32} withName />
    <h1 class="mt-12 text-[1.75rem] leading-tight font-semibold heading">
      {mode === 'setup' ? t('auth.setupTitle') : t('auth.loginTitle')}
    </h1>
    {#if mode === 'setup'}
      <p class="mt-2 text-fg-muted">{t('auth.setupIntro')}</p>
    {/if}
    <div class="mt-6"><FillStrip /></div>

    <form class="mt-8 space-y-4" onsubmit={submit} novalidate>
      <Field label={t('auth.username')}>
        {#snippet children({ id })}
          <TextInput {id} bind:value={username} bind:input={usernameInput} autocomplete="username" required />
        {/snippet}
      </Field>
      <Field label={t('auth.password')} error={rules?.next}>
        {#snippet children({ id, describedBy, invalid })}
          <TextInput
            {id}
            type="password"
            bind:value={password}
            {invalid}
            aria-describedby={describedBy}
            autocomplete={mode === 'setup' ? 'new-password' : 'current-password'}
            required
          />
        {/snippet}
      </Field>
      {#if mode === 'setup'}
        <Field label={t('auth.passwordConfirm')} error={rules?.confirm}>
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
      {:else}
        <Checkbox bind:checked={remember} label={t('auth.remember')} />
      {/if}

      {#if error}
        <Alert>{error}</Alert>
      {/if}

      <Button type="submit" variant="primary" class="w-full" loading={busy} disabled={!canSubmit}>
        {mode === 'setup' ? t('auth.setupSubmit') : t('auth.loginSubmit')}
      </Button>
    </form>
  </div>
</main>
