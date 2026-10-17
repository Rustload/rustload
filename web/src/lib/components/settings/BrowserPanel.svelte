<script lang="ts">
  import AppWindow from '@lucide/svelte/icons/app-window'
  import Bookmark from '@lucide/svelte/icons/bookmark'
  import Copy from '@lucide/svelte/icons/copy'
  import ExternalLink from '@lucide/svelte/icons/external-link'
  import { EXTENSION_URL } from '$lib/config'
  import { t } from '$lib/i18n/index.svelte'
  import { auth } from '$lib/stores/auth.svelte'
  import { pwa } from '$lib/stores/pwa.svelte'
  import { toasts } from '$lib/stores/toasts.svelte'
  import { appWindow } from '$lib/utils/app'
  import { copyText } from '$lib/utils/clipboard'
  import Button from '../ui/Button.svelte'
  import IconButton from '../ui/IconButton.svelte'
  import Panel from '../ui/Panel.svelte'

  // Opens Rustload with the selected text, or the current page, ready to add.
  const base = `${window.location.origin}${window.location.pathname}`
  const bookmarklet = `javascript:(()=>{const s=String(getSelection()).trim();window.open(${JSON.stringify(
    base,
  )}+'?add='+encodeURIComponent(s||location.href),'_blank')})()`
  /** The address the extension uses to reach the app on this computer. */
  const address = window.location.origin

  async function copyAddress() {
    if (await copyText(address)) toasts.success(t('common.copied'))
  }
</script>

<Panel title={t('settings.browser.title')} description={t('settings.browser.intro')}>
  <div class="grid gap-5 sm:grid-cols-2">
    <div>
      <p class="font-medium">{t('settings.browser.extension')}</p>
      <p class="mt-0.5 mb-3 text-small text-fg-muted">{t('settings.browser.extensionHint')}</p>
      <Button size="sm" icon={ExternalLink} href={EXTENSION_URL} external>{t('settings.browser.getExtension')}</Button>
    </div>
    {#if auth.desktop}
      <!-- The app answers on this computer only: the extension needs its address, which can change port. -->
      <div>
        <p class="text-small text-fg-muted">{t('settings.browser.extensionApp', { address })}</p>
        <div class="mt-2 flex items-center gap-1">
          <code class="rounded-control border border-line bg-sunken px-2 py-1 font-mono text-small">{address}</code>
          <IconButton icon={Copy} label={t('common.copy')} size="sm" onclick={copyAddress} />
        </div>
      </div>
    {:else}
      <!-- The bookmarklet opens this address in any browser: it needs a server reachable from it. -->
      <div>
        <p class="font-medium">{t('settings.browser.bookmarklet')}</p>
        <p class="mt-0.5 mb-3 text-small text-fg-muted">{t('settings.browser.bookmarkletHint')}</p>
        <!-- svelte-ignore a11y_invalid_attribute -->
        <a
          href={bookmarklet}
          onclick={(e) => e.preventDefault()}
          draggable="true"
          class="inline-flex h-7 cursor-grab items-center gap-2 rounded-control border border-dashed border-line-strong px-2.5
            text-small font-medium active:cursor-grabbing"
        >
          <Bookmark size={14} aria-hidden="true" />{t('settings.browser.bookmarkletName')}
        </a>
      </div>
    {/if}
  </div>
  {#if pwa.canInstall && !auth.desktop && !appWindow}
    <div class="flex flex-wrap items-center gap-x-4 gap-y-2 border-t border-line pt-4">
      <p class="min-w-0 flex-1 text-small text-fg-muted">{t('settings.browser.installHint')}</p>
      <Button size="sm" icon={AppWindow} onclick={() => pwa.install()}>{t('settings.browser.install')}</Button>
    </div>
  {/if}
</Panel>
