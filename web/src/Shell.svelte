<script lang="ts">
  import GrabberDialog from '$lib/components/add/GrabberDialog.svelte'
  import CommandPalette from '$lib/components/layout/CommandPalette.svelte'
  import MobileNav from '$lib/components/layout/MobileNav.svelte'
  import { NAV_ITEMS } from '$lib/components/layout/nav'
  import ShortcutsDialog from '$lib/components/layout/ShortcutsDialog.svelte'
  import Sidebar from '$lib/components/layout/Sidebar.svelte'
  import TopBar from '$lib/components/layout/TopBar.svelte'
  import ConfirmHost from '$lib/components/ui/ConfirmHost.svelte'
  import { router } from '$lib/router.svelte'
  import { accounts, categories } from '$lib/stores/catalog.svelte'
  import { live } from '$lib/stores/live.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { system } from '$lib/stores/system.svelte'
  import { toasts } from '$lib/stores/toasts.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { containsLinks, isTorrentFile } from '$lib/utils/links'
  import { onMount, type Component } from 'svelte'
  import Dashboard from './pages/Dashboard.svelte'
  import History from './pages/History.svelte'
  import Queue from './pages/Queue.svelte'

  // Pages visited less often load on first use, keeping the first paint light.
  type Lazy = Promise<{ default: Component<any> }>
  const lazy = (load: () => Lazy) => {
    let module: Lazy | undefined
    return () => (module ??= load())
  }
  const loadStats = lazy(() => import('./pages/Stats.svelte'))
  const loadLogs = lazy(() => import('./pages/Logs.svelte'))
  const loadSettings = lazy(() => import('./pages/Settings.svelte'))

  /** Links handed over in the page URL: the PWA share target (`title`/`text`/`url`) or the bookmarklet (`add`). */
  function takeSharedLinks() {
    const params = new URLSearchParams(window.location.search)
    const shared = ['add', 'url', 'text', 'title'].map((key) => params.get(key) ?? '').filter(Boolean)
    if (!shared.length) return
    window.history.replaceState(null, '', window.location.pathname + window.location.hash)
    const text = shared.join('\n')
    if (containsLinks(text)) ui.openAdd(text, [], false)
  }

  onMount(() => {
    takeSharedLinks()
    Promise.all([settings.load(), system.load(), categories.load(), accounts.load(), queue.load()]).catch((err) =>
      toasts.error(err),
    )
    live.start()
    return () => live.stop()
  })

  const page = $derived(router.segments[0] ?? '')
  /** First path segment of each page ('' for the dashboard); settings sections live under "settings". */
  const PAGES = new Set(NAV_ITEMS.map((item) => item.path.split('/')[1]))

  $effect(() => {
    if (!PAGES.has(page)) router.go('/')
  })

  function isTyping(target: EventTarget | null): boolean {
    const el = target as HTMLElement | null
    return !!el?.closest('input, textarea, select, [contenteditable="true"], dialog[open]')
  }

  // "g" then a letter jumps to a page, like in many web apps.
  let awaitingPage = false
  let awaitingTimer: ReturnType<typeof setTimeout> | undefined

  function onkeydown(event: KeyboardEvent) {
    if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === 'k') {
      // Never stack the palette over another dialog.
      if (document.querySelector('dialog[open]:not(.palette)')) return
      event.preventDefault()
      ui.paletteOpen = !ui.paletteOpen
      return
    }
    if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey || isTyping(event.target)) return
    if (awaitingPage) {
      awaitingPage = false
      const target = NAV_ITEMS.find((item) => item.keys[1]?.toLowerCase() === event.key.toLowerCase())
      if (target) {
        event.preventDefault()
        router.go(target.path)
      }
      return
    }
    switch (event.key) {
      case 'n':
      case 'N':
        event.preventDefault()
        ui.openAdd()
        break
      case '/':
        event.preventDefault()
        ui.paletteOpen = true
        break
      case '?':
        event.preventDefault()
        ui.shortcutsOpen = true
        break
      case 'g':
      case 'G':
        awaitingPage = true
        clearTimeout(awaitingTimer)
        awaitingTimer = setTimeout(() => (awaitingPage = false), 1200)
        break
    }
  }

  // Pasting links anywhere outside a field opens the add dialog with them.
  function onpaste(event: ClipboardEvent) {
    if (ui.addOpen || isTyping(event.target)) return
    const text = event.clipboardData?.getData('text') ?? ''
    if (containsLinks(text)) {
      event.preventDefault()
      ui.openAdd(text)
    }
  }

  // Dropping .torrent files or links anywhere opens the add dialog with them. Other files are
  // refused instead of letting the browser navigate away to them.
  const dropTypes = ['Files', 'text/uri-list', 'text/plain']

  function ondragover(event: DragEvent) {
    if (event.dataTransfer?.types.some((type) => dropTypes.includes(type))) event.preventDefault()
  }

  function ondrop(event: DragEvent) {
    const data = event.dataTransfer
    if (!data || event.defaultPrevented) return
    event.preventDefault()
    if (ui.addOpen) return
    const files = [...data.files]
    const torrents = files.filter(isTorrentFile)
    const text = data.getData('text/uri-list') || data.getData('text/plain')
    if (torrents.length || containsLinks(text)) ui.openAdd(containsLinks(text) ? text : '', torrents)
    else if (files.length) toasts.push('warning', t('add.onlyTorrents'))
  }
</script>

<svelte:window {onkeydown} {onpaste} {ondragover} {ondrop} />

<div class="min-h-dvh lg:grid lg:grid-cols-[15rem_minmax(0,1fr)]">
  <Sidebar />
  <div class="min-w-0">
    <TopBar />
    <main class="mx-auto w-full max-w-[88rem] px-4 pt-6 pb-24 sm:px-6 lg:px-8 lg:pb-12">
      {#if page === ''}
        <Dashboard />
      {:else if page === 'queue'}
        <Queue />
      {:else if page === 'history'}
        <History />
      {:else if page === 'stats'}
        {#await loadStats() then { default: Stats }}<Stats />{/await}
      {:else if page === 'logs'}
        {#await loadLogs() then { default: Logs }}<Logs />{/await}
      {:else if page === 'settings'}
        {#await loadSettings() then { default: Settings }}<Settings section={router.segments[1]} />{/await}
      {/if}
    </main>
  </div>
</div>

<MobileNav />
<GrabberDialog />
<CommandPalette />
<ShortcutsDialog />
<ConfirmHost />
