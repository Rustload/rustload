import { createClient } from './api.js'
import { ext } from './browser.js'
import { errorText } from './errors.js'
import { t, tn } from './i18n.js'
import { hasHostPermission, loadSettings } from './settings.js'

/** Badge colours: neutral ink by default, ember (the signal colour) only while something is transferring. */
const NEUTRAL = { color: '#56636f', ink: '#ffffff' }
const EMBER = { color: '#f27318', ink: '#1f0f04' }
const OK = { color: '#1f8a5b', ink: '#ffffff' }

let lastPainted = ''

async function paint({ text = '', colors = NEUTRAL, title }) {
  const key = `${text}|${colors.color}|${title}`
  if (key === lastPainted) return
  lastPainted = key
  try {
    await ext.action.setBadgeText({ text })
    if (text) {
      await ext.action.setBadgeBackgroundColor({ color: colors.color })
      // Not available in every browser version.
      await ext.action.setBadgeTextColor?.({ color: colors.ink })
    }
    await ext.action.setTitle({ title })
  } catch {
    // The badge is decoration: never let it break a caller.
  }
}

/** Packages processed remotely (debrid) or post-processed (checksums, extraction). */
export const PROCESSING = ['debrid', 'verifying', 'extracting']

/** Counts what the queue is doing from `GET /api/packages`. */
export function summarize(packages) {
  let transferring = 0
  let waiting = 0
  let speed = 0
  for (const pkg of packages) {
    if (PROCESSING.includes(pkg.status)) {
      waiting++
      continue
    }
    for (const file of pkg.files ?? []) {
      if (file.status === 'downloading' || file.status === 'resolving') {
        transferring++
        speed += file.speed || 0
      } else if (file.status === 'queued' && !pkg.paused) {
        waiting++
      }
    }
  }
  return { transferring, waiting, speed }
}

/** "2 downloads running, 3 waiting", "Everything is paused"… shared by the popup and the badge. */
export function statusText({ transferring, waiting, paused, scheduleOpen = true }) {
  if (paused) return t('statusPaused')
  const parts = []
  if (transferring > 0) parts.push(tn('statusRunning', transferring))
  else parts.push(scheduleOpen ? t('statusIdle') : t('statusScheduleClosed'))
  if (waiting > 0) parts.push(tn('statusWaiting', waiting))
  return parts.join(', ')
}

/** Number of unfinished downloads; ember only while one is actually transferring. */
export function showSummary({ transferring, waiting, paused = false, scheduleOpen = true }) {
  const count = transferring + waiting
  const moving = transferring > 0 && !paused
  return paint({
    text: count > 0 ? String(Math.min(count, 999)) : '',
    colors: moving ? EMBER : NEUTRAL,
    title: `Rustload — ${statusText({ transferring, waiting, paused, scheduleOpen })}`,
  })
}

/** Brief "+3" confirmation after links were sent, then back to the live count. */
export async function flashBadge(count) {
  lastPainted = ''
  await paint({ text: `+${Math.min(count, 99)}`, colors: OK, title: `Rustload — ${tn('sentLinks', count)}` })
  setTimeout(() => refreshBadge(), 2500)
}

export async function refreshBadge() {
  // Another extension page may have painted since: always repaint.
  lastPainted = ''
  const settings = await loadSettings()
  if (!settings.serverUrl) return paint({ title: `Rustload — ${t('badgeSetup')}` })
  if (!(await hasHostPermission(settings.serverUrl))) {
    return paint({ text: '!', title: `Rustload — ${errorText({ kind: 'permission' }, settings.serverUrl)}` })
  }
  try {
    const client = createClient(settings)
    const [packages, system] = await Promise.all([client.packages(), client.system()])
    return showSummary({ ...summarize(packages), paused: system.paused, scheduleOpen: system.schedule_open })
  } catch (error) {
    return paint({ text: '!', title: `Rustload — ${errorText(error, settings.serverUrl)}` })
  }
}
