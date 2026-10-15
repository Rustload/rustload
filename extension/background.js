/*
 * Background (service worker in Chromium, event page in Firefox): context menus, the toolbar
 * badge, and the page-links picker window.
 */
import { flashBadge, refreshBadge } from './lib/badge.js'
import { ext } from './lib/browser.js'
import { errorText, needsSettings, sendFailedTitle } from './lib/errors.js'
import { t, tn } from './lib/i18n.js'
import { isSendable, parseLinks } from './lib/links.js'
import { collectPageLinks, collectSelectionLinks } from './lib/page-scripts.js'
import { sendLinks } from './lib/send.js'
import { loadSettings } from './lib/settings.js'

const ALARM = 'refresh-badge'
const NOTIFICATION_PREFIX = 'rustload-'
const PICK_PREFIX = 'pick:'
const PICK_TTL_MS = 60 * 60 * 1000

/** Page-level entries only make sense on web pages (not on the new-tab page, settings…). */
const WEB_PAGES = ['http://*/*', 'https://*/*']

const MENUS = [
  { id: 'send-link', contexts: ['link'], title: 'menuSendLink' },
  { id: 'send-selection', contexts: ['selection'], title: 'menuSendSelection' },
  { id: 'send-page-links', contexts: ['page'], title: 'menuSendPageLinks', documentUrlPatterns: WEB_PAGES },
  { id: 'send-page', contexts: ['page'], title: 'menuSendPage', documentUrlPatterns: WEB_PAGES },
]

async function createMenus() {
  await ext.contextMenus.removeAll()
  for (const { title, ...menu } of MENUS) ext.contextMenus.create({ ...menu, title: t(title) })
}

async function setup() {
  await createMenus()
  ext.alarms.create(ALARM, { periodInMinutes: 1 })
  await refreshBadge()
}

// ---- Feedback ------------------------------------------------------------------------------

function notify(title, message) {
  try {
    const created = ext.notifications.create(`${NOTIFICATION_PREFIX}${Date.now()}`, {
      type: 'basic',
      iconUrl: ext.runtime.getURL('icons/icon-128.png'),
      title,
      message: message || title,
    })
    created?.catch?.(() => {})
  } catch {
    // Notifications disabled at the OS level: the badge still tells.
  }
}

function packagesSummary(packages) {
  const [first, ...rest] = packages
  if (!first) return ''
  return rest.length ? tn('morePackages', rest.length, { name: first.name }) : first.name
}

/**
 * Sends from a menu, then confirms with the badge (always) and a notification (if enabled, or
 * when there is something to tell).
 */
async function send(links) {
  const unique = [...new Set(links.filter(isSendable))]
  try {
    const { packages, settings, notice } = await sendLinks(unique)
    await flashBadge(unique.length)
    if (settings.notify || notice) {
      notify(
        t(settings.addPaused ? 'notifyAddedPausedTitle' : 'notifyAddedTitle'),
        notice || packagesSummary(packages) || tn('sentLinks', unique.length),
      )
    }
  } catch (error) {
    const { serverUrl } = await loadSettings()
    notify(sendFailedTitle(error), errorText(error, serverUrl))
    if (needsSettings(error)) ext.runtime.openOptionsPage()
  }
}

// ---- Page scripts --------------------------------------------------------------------------

async function runInTab(tabId, frameId, func) {
  const [injection] = await ext.scripting.executeScript({
    target: { tabId, frameIds: [frameId ?? 0] },
    func,
  })
  return injection?.result
}

async function selectionLinks(info, tab) {
  let found = []
  if (tab?.id != null) {
    try {
      found = (await runInTab(tab.id, info.frameId, collectSelectionLinks)) ?? []
    } catch {
      // Pages scripts can't run on (browser pages, stores, PDF viewer): use the plain text.
    }
  }
  const { links, magnets } = parseLinks([...found, info.selectionText ?? ''].join('\n'))
  return [...links, ...magnets]
}

/** Collects the links of a tab and opens the picker window on them. Throws if the page can't be read. */
async function openPicker(tabId, frameId = 0) {
  const page = await runInTab(tabId, frameId, collectPageLinks)
  if (!page) throw new Error('no result')
  const id = crypto.randomUUID()
  const stored = await ext.storage.session.get(null)
  const expired = Object.entries(stored)
    .filter(([key, value]) => key.startsWith(PICK_PREFIX) && Date.now() - (value?.createdAt ?? 0) > PICK_TTL_MS)
    .map(([key]) => key)
  if (expired.length) await ext.storage.session.remove(expired)
  await ext.storage.session.set({ [`${PICK_PREFIX}${id}`]: { ...page, createdAt: Date.now() } })

  const width = 560
  const height = 680
  const position = {}
  try {
    const current = await ext.windows.getLastFocused()
    if (current?.width && current.left != null) {
      position.left = Math.max(0, Math.round(current.left + (current.width - width) / 2))
      position.top = Math.max(0, Math.round(current.top + Math.min(96, (current.height - height) / 2)))
    }
  } catch {
    // Let the browser place it.
  }
  await ext.windows.create({
    url: ext.runtime.getURL(`picker/picker.html?id=${encodeURIComponent(id)}`),
    type: 'popup',
    width,
    height,
    ...position,
  })
}

// ---- Menus ---------------------------------------------------------------------------------

async function handleMenuClick(info, tab) {
  switch (info.menuItemId) {
    case 'send-link':
      return send([info.linkUrl])
    case 'send-selection':
      return send(await selectionLinks(info, tab))
    case 'send-page':
      return send([info.pageUrl || tab?.url])
    case 'send-page-links':
      try {
        await openPicker(tab.id, info.frameId)
      } catch {
        notify(t('notifyFailedTitle'), t('pageLinksUnavailable'))
      }
      return undefined
    default:
      return undefined
  }
}

// ---- Listeners (registered synchronously so the worker wakes up for them) -------------------

ext.runtime.onInstalled.addListener(async ({ reason }) => {
  await setup()
  if (reason === 'install') ext.runtime.openOptionsPage()
})
ext.runtime.onStartup.addListener(setup)

ext.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === ALARM) refreshBadge()
})

ext.contextMenus.onClicked.addListener((info, tab) => {
  handleMenuClick(info, tab)
})

ext.storage.onChanged.addListener((changes, area) => {
  if (area === 'local' && (changes.serverUrl || changes.apiKey)) refreshBadge()
})
ext.permissions.onAdded.addListener(() => refreshBadge())
ext.permissions.onRemoved.addListener(() => refreshBadge())

ext.notifications.onClicked.addListener(async (id) => {
  if (!id.startsWith(NOTIFICATION_PREFIX)) return
  ext.notifications.clear(id)
  const { serverUrl } = await loadSettings()
  if (serverUrl) ext.tabs.create({ url: serverUrl })
})

/** Messages from the extension's own pages (popup, picker, magnet handler). */
ext.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (sender.id !== ext.runtime.id) return false
  if (message?.type === 'sent') {
    flashBadge(Number(message.count) || 1)
    return false
  }
  if (message?.type === 'open-picker' && Number.isInteger(message.tabId)) {
    openPicker(message.tabId).then(
      () => sendResponse({ ok: true }),
      () => sendResponse({ ok: false }),
    )
    return true
  }
  return false
})

// Alarms may be cleared on browser restart: make sure the badge keeps refreshing.
ext.alarms.get(ALARM).then((alarm) => {
  if (!alarm) ext.alarms.create(ALARM, { periodInMinutes: 1 })
})

// Reachable from the service worker console (and automated tests) to exercise the menu actions.
globalThis.rustload = { handleMenuClick, openPicker, refreshBadge }
