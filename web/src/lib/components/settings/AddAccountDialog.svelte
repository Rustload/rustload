<script lang="ts">
  import ExternalLink from '@lucide/svelte/icons/external-link'
  import Magnet from '@lucide/svelte/icons/magnet'
  import { onDestroy, untrack } from 'svelte'
  import { ApiError, api, type DeviceStart, type Provider } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { auth } from '$lib/stores/auth.svelte'
  import { accounts } from '$lib/stores/catalog.svelte'
  import { errorMessage, toasts } from '$lib/stores/toasts.svelte'
  import Alert from '../ui/Alert.svelte'
  import Button from '../ui/Button.svelte'
  import Dialog from '../ui/Dialog.svelte'
  import Field from '../ui/Field.svelte'
  import Spinner from '../ui/Spinner.svelte'
  import TextInput from '../ui/TextInput.svelte'
  import ProviderMark from './ProviderMark.svelte'

  let { open = $bindable() }: { open: boolean } = $props()

  type Method = 'device' | 'key'

  let provider = $state<Provider | null>(null)
  let method = $state<Method>('key')
  let label = $state('')
  let apiKey = $state('')
  let username = $state('')
  let password = $state('')
  let busy = $state(false)
  let error = $state<string | null>(null)
  let device = $state<DeviceStart | null>(null)
  let deviceExpired = $state(false)
  let pollTimer: ReturnType<typeof setInterval> | undefined

  const methods = $derived(
    provider
      ? [
          {
            id: 'device' as Method,
            title: t('settings.accounts.methodDevice'),
            hint: t('settings.accounts.methodDeviceHint', { provider: provider.name }),
          },
          {
            id: 'key' as Method,
            title: t('settings.accounts.methodKey'),
            hint: t('settings.accounts.methodKeyHint', { provider: provider.name }),
          },
        ]
      : [],
  )

  $effect(() => {
    if (!open) return
    untrack(() => {
      provider = accounts.providers.length === 1 ? accounts.providers[0] : null
      method = provider?.device_auth ? 'device' : 'key'
      reset()
    })
  })

  // Stop polling whenever the dialog closes or goes away.
  $effect(() => {
    if (!open) stopPolling()
  })
  onDestroy(stopPolling)

  const credentials = $derived(provider?.auth === 'credentials')
  const ready = $derived(credentials ? !!username.trim() && !!password : !!apiKey.trim())
  const groups = $derived(
    (['debrid', 'hoster'] as const)
      .map((kind) => ({ kind, providers: accounts.providers.filter((p) => (p.kind ?? 'debrid') === kind) }))
      .filter((g) => g.providers.length),
  )

  function reset() {
    label = ''
    apiKey = ''
    username = ''
    password = ''
    error = null
    device = null
    deviceExpired = false
    stopPolling()
  }

  function choose(next: Provider) {
    provider = next
    method = next.device_auth ? 'device' : 'key'
    reset()
  }

  function done(accountLabel: string) {
    toasts.success(t('settings.accounts.connected', { provider: accountLabel }))
    open = false
  }

  async function submitKey(event: SubmitEvent) {
    event.preventDefault()
    if (!provider || !ready) return
    busy = true
    error = null
    try {
      const account = await api.accounts.create(
        credentials
          ? { provider: provider.id, username: username.trim(), password, label: label.trim() }
          : { provider: provider.id, api_key: apiKey.trim(), label: label.trim() },
      )
      accounts.replace(account)
      done(provider.name)
    } catch (err) {
      error =
        err instanceof ApiError && err.code === 'INVALID_API_KEY'
          ? t(credentials ? 'settings.accounts.invalidLogin' : 'settings.accounts.invalidKey', {
              provider: provider.name,
            })
          : errorMessage(err)
    } finally {
      busy = false
    }
  }

  async function startDevice() {
    if (!provider) return
    const current = provider
    busy = true
    error = null
    deviceExpired = false
    try {
      device = await api.accounts.deviceStart(current.id)
      const token = device.poll_token
      stopPolling()
      pollTimer = setInterval(async () => {
        try {
          const result = await api.accounts.devicePoll(current.id, token)
          if (result.status === 'activated' && result.account) {
            stopPolling()
            accounts.replace(result.account)
            done(current.name)
          } else if (result.status === 'expired') {
            stopPolling()
            deviceExpired = true
          }
        } catch (err) {
          stopPolling()
          error = errorMessage(err)
        }
      }, 5000)
    } catch (err) {
      error = errorMessage(err)
    } finally {
      busy = false
    }
  }

  function stopPolling() {
    clearInterval(pollTimer)
    pollTimer = undefined
  }
</script>

<Dialog bind:open title={t('settings.accounts.add')}>
  {#if !provider}
    {#each groups as group (group.kind)}
      <h3 class="mb-1 font-medium">{t(`settings.accounts.kinds.${group.kind}`)}</h3>
      <p class="mb-3 text-small text-fg-muted">{t(`settings.accounts.kindHints.${group.kind}`)}</p>
      <ul class="mb-6 grid gap-2 last:mb-0 sm:grid-cols-2">
        {#each group.providers as p (p.id)}
          <li>
            <button
              type="button"
              onclick={() => choose(p)}
              class="flex w-full items-center gap-3 rounded-control border border-line bg-sunken px-3 py-2.5 text-left
              transition-colors hover:border-line-strong hover:bg-raised"
            >
              <ProviderMark name={p.name} />
              <span class="min-w-0 flex-1">
                <span class="block font-medium">{p.name}</span>
                <span class="block truncate text-xs text-fg-muted">{p.website.replace(/^https?:\/\//, '')}</span>
              </span>
              {#if p.magnets}
                <Magnet size={15} class="text-fg-faint" aria-label={t('settings.accounts.magnets')} />
              {/if}
            </button>
          </li>
        {/each}
      </ul>
    {/each}
  {:else}
    <div class="mb-5 flex items-center gap-3">
      <ProviderMark name={provider.name} />
      <div class="min-w-0 flex-1">
        <p class="font-medium">{provider.name}</p>
        <a
          href={provider.website}
          target="_blank"
          rel="noopener noreferrer"
          class="text-xs text-fg-muted hover:underline"
        >
          {provider.website.replace(/^https?:\/\//, '')}
        </a>
      </div>
      {#if accounts.providers.length > 1}
        <Button size="sm" variant="ghost" onclick={() => (provider = null)}>{t('common.back')}</Button>
      {/if}
    </div>

    {#if provider.device_auth}
      <fieldset class="mb-5">
        <legend class="mb-2 font-medium">{t('settings.accounts.method')}</legend>
        <div class="grid gap-2 sm:grid-cols-2">
          {#each methods as option (option.id)}
            <label
              class="flex cursor-pointer gap-3 rounded-control border px-3 py-2.5 transition-colors
                {method === option.id ? 'border-fg bg-raised' : 'border-line bg-sunken hover:border-line-strong'}"
            >
              <input type="radio" name="method" value={option.id} bind:group={method} class="mt-1 accent-[var(--fg)]" />
              <span>
                <span class="block font-medium">{option.title}</span>
                <span class="block text-xs text-fg-muted">{option.hint}</span>
              </span>
            </label>
          {/each}
        </div>
      </fieldset>
    {/if}

    {#if method === 'key'}
      <form id="account-form" class="space-y-4" onsubmit={submitKey}>
        <Field label={t('settings.accounts.label')} optional>
          {#snippet children({ id })}
            <TextInput {id} bind:value={label} placeholder={t('settings.accounts.labelPlaceholder')} />
          {/snippet}
        </Field>
        {#if credentials}
          <Field label={t('settings.accounts.username')}>
            {#snippet children({ id })}
              <TextInput {id} bind:value={username} required autocomplete="off" spellcheck={false} />
            {/snippet}
          </Field>
          <Field
            label={t('settings.accounts.password')}
            hint={auth.desktop ? t('settings.accounts.passwordHintApp') : t('settings.accounts.passwordHint')}
          >
            {#snippet children({ id, describedBy })}
              <TextInput
                {id}
                type="password"
                bind:value={password}
                required
                autocomplete="new-password"
                aria-describedby={describedBy}
              />
            {/snippet}
          </Field>
        {:else}
          <Field
            label={t('settings.accounts.apiKey')}
            hint={t('settings.accounts.apiKeyHint', { provider: provider.name })}
          >
            {#snippet children({ id, describedBy })}
              <TextInput
                {id}
                type="password"
                bind:value={apiKey}
                mono
                required
                autocomplete="off"
                spellcheck={false}
                aria-describedby={describedBy}
              />
            {/snippet}
          </Field>
          <Button size="sm" variant="ghost" icon={ExternalLink} href={provider.api_key_url} external class="-ml-2.5">
            {t('settings.accounts.apiKeyLink')}
          </Button>
        {/if}
      </form>
    {:else if device && !deviceExpired}
      <div class="rounded-control border border-line bg-sunken px-5 py-5 text-center">
        <p class="text-fg-muted">{t('settings.accounts.deviceStep', { provider: provider.name })}</p>
        <p class="my-4 readout text-5xl tracking-[0.12em] select-all">{device.user_code}</p>
        <Button variant="secondary" icon={ExternalLink} href={device.verification_url} external>
          {t('settings.accounts.deviceOpen', { provider: provider.name })}
        </Button>
        <p class="mt-4 flex items-center justify-center gap-2 text-small text-fg-muted" aria-live="polite">
          <Spinner size={14} />
          {t('settings.accounts.deviceWaiting')}
        </p>
      </div>
    {:else if deviceExpired}
      <p class="mb-3 text-warn">{t('settings.accounts.deviceExpired')}</p>
    {/if}

    {#if error}
      <Alert class="mt-4">{error}</Alert>
    {/if}
  {/if}

  {#snippet footer()}
    <Button onclick={() => (open = false)}>{t('common.cancel')}</Button>
    {#if provider && method === 'key'}
      <Button type="submit" form="account-form" variant="primary" loading={busy} disabled={!ready}>
        {t('settings.accounts.add')}
      </Button>
    {:else if provider && method === 'device' && (!device || deviceExpired)}
      <Button variant="primary" loading={busy} onclick={startDevice}>
        {deviceExpired ? t('settings.accounts.deviceRestart') : t('settings.accounts.methodDevice')}
      </Button>
    {/if}
  {/snippet}
</Dialog>
