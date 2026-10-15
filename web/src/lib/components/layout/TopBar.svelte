<script lang="ts">
  import ArrowDown from '@lucide/svelte/icons/arrow-down'
  import CirclePause from '@lucide/svelte/icons/circle-pause'
  import Clock from '@lucide/svelte/icons/clock'
  import Gauge from '@lucide/svelte/icons/gauge'
  import HardDrive from '@lucide/svelte/icons/hard-drive'
  import Pause from '@lucide/svelte/icons/pause'
  import Play from '@lucide/svelte/icons/play'
  import Plus from '@lucide/svelte/icons/plus'
  import Search from '@lucide/svelte/icons/search'
  import WifiOff from '@lucide/svelte/icons/wifi-off'
  import { t } from '$lib/i18n/index.svelte'
  import { router } from '$lib/router.svelte'
  import { live } from '$lib/stores/live.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { system } from '$lib/stores/system.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { formatSpeed, splitSpeed } from '$lib/utils/format'
  import Badge from '../ui/Badge.svelte'
  import Button from '../ui/Button.svelte'
  import IconButton from '../ui/IconButton.svelte'
  import { buttonClass } from '../ui/styles'
  import Logo from './Logo.svelte'

  const speed = $derived(splitSpeed(system.speed))
  // Outside the dashboard the bar also shows the speed: on phones the Add button keeps only its icon.
  const showSpeed = $derived(router.path !== '/')
  const limit = $derived(settings.current?.downloads)
  const limitActive = $derived(settings.speedLimit > 0)
  const limitLabel = $derived(
    limitActive ? t('shell.limitOn', { speed: formatSpeed(settings.speedLimit) }) : t('shell.limitOff'),
  )
  const scheduleStart = $derived(settings.current?.schedule.start ?? '')

  function toggleLimit() {
    if (!limit) return
    if (!limit.speed_limit_enabled && limit.speed_limit_kib <= 0) {
      router.go('/settings/downloads')
      return
    }
    settings.toggleSpeedLimit()
  }
</script>

<header class="sticky top-0 z-30 flex h-14 items-center gap-3 border-b border-line bg-bg px-4 sm:px-6 lg:px-8">
  <a href={router.href('/')} class="lg:hidden" aria-label="Rustload">
    <Logo size={24} />
  </a>

  {#if showSpeed}
    <div class="flex items-center gap-1.5" title={t('shell.totalSpeed')}>
      <ArrowDown size={15} class={system.speed > 0 ? 'text-signal-ink' : 'text-fg-faint'} aria-hidden="true" />
      <span class="sr-only">{t('shell.totalSpeed')}</span>
      <span class="min-w-[3.5ch] text-right num text-[1.0625rem] font-medium">{speed.value}</span>
      <span class="text-xs text-fg-muted">{speed.unit}</span>
    </div>
  {/if}

  <div class="flex min-w-0 items-center gap-2">
    {#if live.lost}
      <Badge tone="warn" icon={WifiOff} label={t('shell.reconnecting')} />
    {:else if system.paused}
      <Badge tone="muted" icon={CirclePause} label={t('shell.pausedAll')} />
    {:else if system.diskLow}
      <Badge tone="bad" icon={HardDrive} label={t('shell.diskLow')} />
    {:else if !system.scheduleOpen}
      <Badge
        tone="muted"
        icon={Clock}
        label={t('shell.scheduleClosedShort', { time: scheduleStart })}
        title={t('shell.scheduleClosed', { time: scheduleStart })}
      />
    {:else if system.stats?.speed_rule_active}
      <Badge
        tone="muted"
        icon={Gauge}
        label={t('shell.ruleActive', { speed: formatSpeed(system.stats.speed_limit) })}
        title={t('shell.ruleActiveHint')}
      />
    {/if}
  </div>

  <div class="ml-auto flex items-center gap-1.5 sm:gap-2">
    <IconButton icon={Search} label={t('palette.title')} class="lg:hidden" onclick={() => (ui.paletteOpen = true)} />
    {#if limit}
      <button
        type="button"
        aria-pressed={limitActive}
        title={t('shell.limitToggle')}
        onclick={toggleLimit}
        class={buttonClass('ghost', 'md', `px-2.5 ${limitActive ? 'text-fg' : ''}`)}
      >
        <Gauge size={16} aria-hidden="true" />
        <span class="hidden md:inline {limitActive ? 'num' : ''}">{limitLabel}</span>
        <span class="sr-only md:hidden">{limitLabel}</span>
      </button>
    {/if}

    <button
      type="button"
      onclick={() => system.togglePause()}
      class={buttonClass('secondary', 'md', 'px-2.5 sm:px-3.5')}
    >
      {#if system.paused}
        <Play size={16} aria-hidden="true" />
        <span class="hidden sm:inline">{t('shell.resumeAll')}</span>
        <span class="sr-only sm:hidden">{t('shell.resumeAll')}</span>
      {:else}
        <Pause size={16} aria-hidden="true" />
        <span class="hidden sm:inline">{t('shell.pauseAll')}</span>
        <span class="sr-only sm:hidden">{t('shell.pauseAll')}</span>
      {/if}
    </button>

    <Button
      variant="primary"
      icon={Plus}
      title={t('shell.addHint')}
      class={showSpeed ? 'max-sm:px-2.5' : ''}
      onclick={() => ui.openAdd()}
    >
      <span class={showSpeed ? 'max-sm:sr-only' : ''}>{t('shell.add')}</span>
    </Button>
  </div>
</header>
