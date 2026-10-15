<script lang="ts">
  import AppWindow from '@lucide/svelte/icons/app-window'
  import Bookmark from '@lucide/svelte/icons/bookmark'
  import ExternalLink from '@lucide/svelte/icons/external-link'
  import { EXTENSION_URL } from '$lib/config'
  import { t } from '$lib/i18n/index.svelte'
  import { pwa } from '$lib/stores/pwa.svelte'
  import Button from '../ui/Button.svelte'
  import Panel from '../ui/Panel.svelte'

  // Opens Rustload with the selected text, or the current page, ready to add.
  const base = `${window.location.origin}${window.location.pathname}`
  const bookmarklet = `javascript:(()=>{const s=String(getSelection()).trim();window.open(${JSON.stringify(
    base,
  )}+'?add='+encodeURIComponent(s||location.href),'_blank')})()`
</script>

<Panel title={t('settings.browser.title')} description={t('settings.browser.intro')}>
  <div class="grid gap-5 sm:grid-cols-2">
    <div>
      <p class="font-medium">{t('settings.browser.extension')}</p>
      <p class="mt-0.5 mb-3 text-small text-fg-muted">{t('settings.browser.extensionHint')}</p>
      <Button size="sm" icon={ExternalLink} href={EXTENSION_URL} external>{t('settings.browser.getExtension')}</Button>
    </div>
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
  </div>
  {#if pwa.canInstall}
    <div class="flex flex-wrap items-center gap-x-4 gap-y-2 border-t border-line pt-4">
      <p class="min-w-0 flex-1 text-small text-fg-muted">{t('settings.browser.installHint')}</p>
      <Button size="sm" icon={AppWindow} onclick={() => pwa.install()}>{t('settings.browser.install')}</Button>
    </div>
  {/if}
</Panel>
