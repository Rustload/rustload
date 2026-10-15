<script lang="ts">
  import type { LucideIcon } from '@lucide/svelte'
  import Gauge from '@lucide/svelte/icons/gauge'
  import History from '@lucide/svelte/icons/history'
  import Keyboard from '@lucide/svelte/icons/keyboard'
  import LogOut from '@lucide/svelte/icons/log-out'
  import Moon from '@lucide/svelte/icons/moon'
  import Package from '@lucide/svelte/icons/package'
  import Pause from '@lucide/svelte/icons/pause'
  import Play from '@lucide/svelte/icons/play'
  import Plus from '@lucide/svelte/icons/plus'
  import RotateCcw from '@lucide/svelte/icons/rotate-ccw'
  import Search from '@lucide/svelte/icons/search'
  import Sun from '@lucide/svelte/icons/sun'
  import { tick } from 'svelte'
  import { api, type Package as Pkg } from '$lib/api'
  import { t, type MessageKey } from '$lib/i18n/index.svelte'
  import { router } from '$lib/router.svelte'
  import { auth } from '$lib/stores/auth.svelte'
  import { history } from '$lib/stores/history.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { system } from '$lib/stores/system.svelte'
  import { theme } from '$lib/stores/theme.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { formatBytes, formatDate } from '$lib/utils/format'
  import { packageProgress } from '$lib/utils/packages'
  import { NAV_ITEMS, SETTINGS_SECTIONS } from './nav'

  type Command = {
    id: string
    group: 'actions' | 'goto' | 'queue' | 'history'
    label: string
    icon: LucideIcon
    hint?: string
    keys?: string[]
    run: () => void
  }

  let dialog: HTMLDialogElement | undefined = $state()
  let input: HTMLInputElement | undefined = $state()
  let query = $state('')
  let active = $state(0)
  let found = $state<Pkg[]>([])
  let searchTimer: ReturnType<typeof setTimeout> | undefined

  $effect(() => {
    if (!dialog) return
    if (ui.paletteOpen && !dialog.open) {
      query = ''
      active = 0
      found = []
      dialog.showModal()
      tick().then(() => input?.focus())
    } else if (!ui.paletteOpen && dialog.open) {
      dialog.close()
    }
  })

  // History is searched on the server, a moment after typing stops.
  $effect(() => {
    const q = query.trim()
    clearTimeout(searchTimer)
    if (q.length < 2) {
      found = []
      return
    }
    searchTimer = setTimeout(async () => {
      try {
        const { items } = await api.history.list(q, 5, 0)
        if (query.trim() === q) found = items
      } catch {
        found = []
      }
    }, 200)
    return () => clearTimeout(searchTimer)
  })

  /** "Paramètres" matches "parametres", "File d’attente" matches "file d'attente". */
  function fold(text: string): string {
    return text.normalize('NFD').replace(/\p{M}/gu, '').replace(/[’‘]/g, "'").toLowerCase()
  }

  function go(path: string) {
    router.go(path)
  }

  const commands = $derived.by((): Command[] => {
    const failed = queue.packages.filter((p) => p.status === 'failed')
    const limit = settings.current?.downloads
    const list: Command[] = [
      {
        id: 'add',
        group: 'actions',
        label: t('palette.add'),
        icon: Plus,
        keys: ['N'],
        run: () => ui.openAdd(),
      },
      {
        id: 'pause',
        group: 'actions',
        label: system.paused ? t('shell.resumeAll') : t('shell.pauseAll'),
        icon: system.paused ? Play : Pause,
        run: () => system.togglePause(),
      },
    ]
    if (limit && limit.speed_limit_kib > 0) {
      list.push({
        id: 'limit',
        group: 'actions',
        label: limit.speed_limit_enabled ? t('palette.limitOff') : t('palette.limitOn'),
        icon: Gauge,
        run: () => settings.toggleSpeedLimit(),
      })
    }
    if (failed.length) {
      list.push({
        id: 'retry',
        group: 'actions',
        label: t('palette.retryFailed', { count: failed.length }),
        icon: RotateCcw,
        run: () => queue.run(() => Promise.all(failed.map((p) => api.packages.retry(p.id)))),
      })
    }
    list.push(
      {
        id: 'theme',
        group: 'actions',
        label: theme.dark ? t('shell.themeLight') : t('shell.themeDark'),
        icon: theme.dark ? Sun : Moon,
        run: () => settings.setTheme(theme.dark ? 'light' : 'dark'),
      },
      {
        id: 'shortcuts',
        group: 'actions',
        label: t('palette.shortcuts'),
        icon: Keyboard,
        keys: ['?'],
        run: () => (ui.shortcutsOpen = true),
      },
    )
    if (auth.enabled && auth.username) {
      list.push({ id: 'logout', group: 'actions', label: t('shell.signOut'), icon: LogOut, run: () => auth.logout() })
    }

    for (const item of NAV_ITEMS) {
      list.push({
        id: `nav:${item.path}`,
        group: 'goto',
        label: t(item.label),
        icon: item.icon,
        keys: item.keys,
        run: () => go(item.path),
      })
    }
    for (const section of SETTINGS_SECTIONS) {
      list.push({
        id: `settings:${section.id}`,
        group: 'goto',
        label: t('palette.settingsSection', { section: t(`settings.sections.${section.id}` as MessageKey) }),
        icon: section.icon,
        run: () => go(`/settings/${section.id}`),
      })
    }

    const q = fold(query.trim())
    if (q) {
      for (const pkg of queue.packages.filter((p) => fold(p.name).includes(q)).slice(0, 6)) {
        const progress = packageProgress(pkg)
        list.push({
          id: `queue:${pkg.id}`,
          group: 'queue',
          label: pkg.name,
          icon: Package,
          hint: `${t(`status.package.${pkg.status}`)}, ${formatBytes(progress.total)}`,
          run: () => ui.revealPackage(pkg.id),
        })
      }
      for (const pkg of found) {
        list.push({
          id: `history:${pkg.id}`,
          group: 'history',
          label: pkg.name,
          icon: History,
          hint: formatDate(pkg.finished_at),
          run: () => {
            history.search(pkg.name)
            go('/history')
          },
        })
      }
    }
    return list
  })

  const results = $derived.by(() => {
    const q = fold(query.trim())
    if (!q) return commands.filter((c) => c.group === 'actions' || c.group === 'goto').slice(0, 14)
    const score = (c: Command) => {
      const label = fold(c.label)
      if (c.group === 'queue' || c.group === 'history') return 2
      if (label.startsWith(q)) return 0
      if (label.split(/\s+/).some((word) => word.startsWith(q))) return 1
      return label.includes(q) ? 3 : -1
    }
    return commands
      .map((c) => ({ c, s: score(c) }))
      .filter(({ s }) => s >= 0)
      .sort((a, b) => a.s - b.s)
      .map(({ c }) => c)
  })

  const GROUP_ORDER = ['actions', 'goto', 'queue', 'history'] as const
  const sections = $derived(
    GROUP_ORDER.map((group) => ({ group, items: results.filter((c) => c.group === group) })).filter(
      (s) => s.items.length,
    ),
  )
  const ordered = $derived(sections.flatMap((s) => s.items))

  $effect(() => {
    if (active >= ordered.length) active = Math.max(0, ordered.length - 1)
  })

  function run(command: Command) {
    ui.paletteOpen = false
    command.run()
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.isComposing) return
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      const step = event.key === 'ArrowDown' ? 1 : -1
      active = (active + step + ordered.length) % Math.max(1, ordered.length)
      tick().then(() => dialog?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' }))
    } else if (event.key === 'Enter') {
      event.preventDefault()
      const command = ordered[active]
      if (command) run(command)
    }
  }

  function onpointerdown(event: PointerEvent) {
    if (event.target === dialog) ui.paletteOpen = false
  }
</script>

<dialog
  bind:this={dialog}
  onclose={() => (ui.paletteOpen = false)}
  {onpointerdown}
  aria-label={t('palette.title')}
  class="palette mx-auto mt-[12vh] mb-auto w-[calc(100%-1.5rem)] max-w-xl overflow-hidden rounded-panel border border-line
    bg-raised p-0 text-fg shadow-pop backdrop:bg-overlay"
>
  {#if ui.paletteOpen}
    <div class="flex h-13 items-center gap-3 border-b border-line px-4">
      <Search size={17} class="shrink-0 text-fg-faint" aria-hidden="true" />
      <input
        bind:this={input}
        bind:value={query}
        {onkeydown}
        oninput={() => (active = 0)}
        role="combobox"
        aria-expanded="true"
        aria-controls="palette-results"
        aria-activedescendant={ordered[active] ? `palette-${ordered[active].id}` : undefined}
        aria-autocomplete="list"
        placeholder={t('palette.placeholder')}
        spellcheck={false}
        autocomplete="off"
        class="h-full min-w-0 flex-1 bg-transparent text-lead placeholder:text-fg-faint focus:outline-none"
      />
      <kbd class="hidden rounded-chip border border-line px-1.5 py-0.5 text-micro text-fg-faint sm:block"
        >{t('palette.escKey')}</kbd
      >
    </div>

    <div id="palette-results" role="listbox" class="max-h-[min(60vh,28rem)] overflow-y-auto p-1.5">
      {#if ordered.length === 0}
        <p class="px-3 py-6 text-center text-fg-muted">{t('palette.empty', { query: query.trim() })}</p>
      {/if}
      {#each sections as section (section.group)}
        <div role="group" aria-labelledby="palette-group-{section.group}" class="pb-1">
          <p id="palette-group-{section.group}" class="px-3 pt-2 pb-1 text-xs text-fg-faint">
            {t(`palette.groups.${section.group}`)}
          </p>
          {#each section.items as command (command.id)}
            {@const selected = ordered[active]?.id === command.id}
            <!-- Keyboard users pick options from the search field (arrows and Enter). -->
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div
              id="palette-{command.id}"
              role="option"
              tabindex="-1"
              aria-selected={selected}
              onclick={() => run(command)}
              onpointermove={() => (active = ordered.indexOf(command))}
              class="flex h-9 cursor-pointer items-center gap-3 rounded-control px-3 {selected
                ? 'bg-sunken text-fg'
                : 'text-fg-muted'}"
            >
              <command.icon size={16} class="shrink-0 {selected ? 'text-fg' : 'text-fg-faint'}" aria-hidden="true" />
              <span class="min-w-0 flex-1 truncate">{command.label}</span>
              {#if command.hint}
                <span class="shrink-0 num text-xs text-fg-faint">{command.hint}</span>
              {/if}
              {#each command.keys ?? [] as key (key)}
                <kbd class="rounded-chip border border-line px-1.5 text-micro text-fg-faint">{key}</kbd>
              {/each}
            </div>
          {/each}
        </div>
      {/each}
    </div>
  {/if}
</dialog>

<style>
  .palette[open] {
    animation: palette-in 120ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  @keyframes palette-in {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
</style>
