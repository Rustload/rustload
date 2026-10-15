import type { LucideIcon } from '@lucide/svelte'
import Bell from '@lucide/svelte/icons/bell'
import CalendarClock from '@lucide/svelte/icons/calendar-clock'
import ChartColumn from '@lucide/svelte/icons/chart-column'
import Download from '@lucide/svelte/icons/download'
import FileArchive from '@lucide/svelte/icons/file-archive'
import FolderTree from '@lucide/svelte/icons/folder-tree'
import Gauge from '@lucide/svelte/icons/gauge'
import History from '@lucide/svelte/icons/history'
import Info from '@lucide/svelte/icons/info'
import KeyRound from '@lucide/svelte/icons/key-round'
import ListOrdered from '@lucide/svelte/icons/list-ordered'
import MonitorPlay from '@lucide/svelte/icons/monitor-play'
import Plug from '@lucide/svelte/icons/plug'
import ScrollText from '@lucide/svelte/icons/scroll-text'
import Settings from '@lucide/svelte/icons/settings'
import Shield from '@lucide/svelte/icons/shield'
import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal'
import Stethoscope from '@lucide/svelte/icons/stethoscope'
import type { MessageKey } from '$lib/i18n/index.svelte'

/**
 * `short` replaces `label` in the phone navigation bar when the full label is too long; `mobile`
 * pages appear in that bar (the others stay reachable from the dashboard and the command palette).
 * `keys` is the "g then key" shortcut.
 */
export type NavItem = {
  path: string
  label: MessageKey
  short?: MessageKey
  icon: LucideIcon
  keys: string[]
  mobile: boolean
}

export const NAV_ITEMS: NavItem[] = [
  { path: '/', label: 'nav.dashboard', short: 'nav.dashboardShort', icon: Gauge, keys: ['G', 'D'], mobile: true },
  { path: '/queue', label: 'nav.queue', icon: ListOrdered, keys: ['G', 'Q'], mobile: true },
  { path: '/history', label: 'nav.history', icon: History, keys: ['G', 'H'], mobile: true },
  { path: '/stats', label: 'nav.stats', icon: ChartColumn, keys: ['G', 'T'], mobile: false },
  { path: '/logs', label: 'nav.logs', icon: ScrollText, keys: ['G', 'L'], mobile: true },
  { path: '/settings', label: 'nav.settings', icon: Settings, keys: ['G', 'S'], mobile: true },
]

export type SettingsSectionId =
  | 'general'
  | 'downloads'
  | 'schedule'
  | 'folders'
  | 'postprocess'
  | 'accounts'
  | 'media'
  | 'notifications'
  | 'integrations'
  | 'system'
  | 'security'
  | 'about'

export const SETTINGS_SECTIONS: { id: SettingsSectionId; icon: LucideIcon }[] = [
  { id: 'general', icon: SlidersHorizontal },
  { id: 'downloads', icon: Download },
  { id: 'schedule', icon: CalendarClock },
  { id: 'folders', icon: FolderTree },
  { id: 'postprocess', icon: FileArchive },
  { id: 'accounts', icon: KeyRound },
  { id: 'media', icon: MonitorPlay },
  { id: 'notifications', icon: Bell },
  { id: 'integrations', icon: Plug },
  { id: 'system', icon: Stethoscope },
  { id: 'security', icon: Shield },
  { id: 'about', icon: Info },
]
