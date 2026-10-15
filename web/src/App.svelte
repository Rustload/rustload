<script lang="ts">
  import ServerOff from '@lucide/svelte/icons/server-off'
  import Logo from '$lib/components/layout/Logo.svelte'
  import EmptyState from '$lib/components/ui/EmptyState.svelte'
  import Spinner from '$lib/components/ui/Spinner.svelte'
  import Toasts from '$lib/components/ui/Toasts.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { auth } from '$lib/stores/auth.svelte'
  import AuthScreen from './pages/AuthScreen.svelte'
  import Shell from './Shell.svelte'

  auth.load()

  // Keep retrying while the server is unreachable at startup.
  $effect(() => {
    if (auth.phase !== 'unreachable') return
    const timer = setInterval(() => auth.load(), 5000)
    return () => clearInterval(timer)
  })
</script>

{#if auth.phase === 'loading'}
  <div class="grid min-h-dvh place-items-center text-fg-faint">
    <div class="flex flex-col items-center gap-4">
      <Logo size={36} />
      <Spinner size={18} />
    </div>
  </div>
{:else if auth.phase === 'unreachable'}
  <div class="grid min-h-dvh place-items-center">
    <EmptyState icon={ServerOff} title="Rustload" body={t('errors.network')}>
      {#snippet action()}
        <Spinner size={18} class="text-fg-faint" />
      {/snippet}
    </EmptyState>
  </div>
{:else if auth.phase === 'setup' || auth.phase === 'login'}
  <AuthScreen mode={auth.phase} />
{:else}
  <Shell />
{/if}

<Toasts />
