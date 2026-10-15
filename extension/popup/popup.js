import { ApiError, createClient } from '../lib/api.js'
import { PROCESSING, showSummary, statusText, summarize } from '../lib/badge.js'
import { ext } from '../lib/browser.js'
import { $, el, showFeedback as fillFeedback, withSpinner } from '../lib/dom.js'
import { errorHeadline, errorText, needsSettings } from '../lib/errors.js'
import {
  displayName,
  formatBytes,
  formatDuration,
  formatPercent,
  formatSpeed,
  niceBytesCeil,
  splitSpeed,
} from '../lib/format.js'
import { localize, t, tn } from '../lib/i18n.js'
import { hydrateIcons, icon } from '../lib/icons.js'
import { parseLinks } from '../lib/links.js'
import { sendLinks } from '../lib/send.js'
import { displayHost, hasHostPermission, loadSettings } from '../lib/settings.js'

/** One bar per second (per poll in fallback mode) over the last ~minute. */
const SAMPLE_COUNT = 48
const POLL_MS = 2000
/** No event at all for this long means the stream is stuck (e.g. a buffering proxy). */
const WATCHDOG_MS = 5000
const STREAM_RETRY_MS = 15000
const REFETCH_DEBOUNCE_MS = 250

const state = {
  settings: null,
  client: null,
  packages: [],
  /** Global flags: `paused`, `schedule_open` (from /api/system, then from `stats`). */
  system: null,
  /** Last `stats` event while streaming, null while polling. */
  stats: null,
  samples: [],
  /** 'live' (Server-Sent Events) | 'polling' (fallback) | 'idle' */
  mode: 'idle',
  connected: false,
}

// ---- Views ---------------------------------------------------------------------------------

function showView(name) {
  $('view-live').hidden = name !== 'live'
  $('view-message').hidden = name !== 'message'
  const configured = Boolean(state.settings?.serverUrl)
  $('footer').hidden = !configured
  $('toggle-pause').hidden = name !== 'live'
  $('open-app').hidden = !configured
}

/**
 * Full-size message: `{ icon, title, body, primary: [label, action], secondary, spinner }`.
 */
function showMessage({ icon: iconName, title, body = '', primary, secondary, spinner = false }) {
  const well = $('message-icon')
  well.replaceChildren()
  if (spinner) {
    const spin = document.createElement('span')
    spin.className = 'spinner'
    well.append(spin)
  } else {
    well.append(icon(iconName, { size: 20, strokeWidth: 1.75 }))
  }
  $('message-title').textContent = title
  $('message-body').textContent = body
  $('message-body').hidden = !body
  for (const [id, action] of [
    ['message-primary', primary],
    ['message-secondary', secondary],
  ]) {
    const button = $(id)
    button.hidden = !action
    if (action) {
      button.textContent = action[0]
      button.onclick = action[1]
    }
  }
  showView('message')
}

function showError(error) {
  const { title, icon: iconName } = errorHeadline(error)
  const body =
    error?.kind === 'unconfigured'
      ? t('setupBody')
      : error?.kind === 'permission'
        ? t('permissionBody', { host: displayHost(state.settings.serverUrl) })
        : errorText(error, state.settings?.serverUrl)
  const settingsAction = [t('openSettings'), openSettings]
  const retryAction = [t('retry'), () => start()]
  showMessage({
    icon: iconName,
    title,
    body,
    primary: needsSettings(error) ? settingsAction : retryAction,
    secondary: needsSettings(error) ? undefined : settingsAction,
  })
  setConnection(false)
}

function setConnection(connected) {
  state.connected = connected
  const dot = $('conn-dot')
  dot.classList.toggle('is-live', connected && state.mode === 'live')
  dot.classList.toggle('is-warn', !connected)
  const host = displayHost(state.settings?.serverUrl ?? '')
  $('conn').title = !connected
    ? errorText({ kind: 'unreachable' }, state.settings?.serverUrl)
    : state.mode === 'live'
      ? t('connLive', { host })
      : t('connPolling', { host })
}

// ---- Rendering -----------------------------------------------------------------------------

function currentCounts() {
  const summary = summarize(state.packages)
  if (state.stats) {
    return {
      transferring: state.stats.active,
      waiting: state.stats.queued,
      speed: state.stats.speed,
    }
  }
  return summary
}

function renderMeter() {
  const { transferring, waiting, speed } = currentCounts()
  const paused = Boolean(state.system?.paused)
  const scheduleOpen = state.system?.schedule_open !== false

  const readout = splitSpeed(speed)
  $('speed-value').textContent = readout.value
  $('speed-unit').textContent = readout.unit
  $('speed-arrow').classList.toggle('is-moving', speed > 0)

  $('status-text').textContent = statusText({ transferring, waiting, paused, scheduleOpen })
  const statusIcon = $('status-icon')
  statusIcon.replaceChildren()
  if (paused) statusIcon.append(icon('circle-pause', { size: 14 }))
  else if (!scheduleOpen) statusIcon.append(icon('clock', { size: 14 }))

  renderSpark()
  showSummary({ transferring, waiting, paused, scheduleOpen })
}

function pushSample(speed) {
  state.samples.push(Math.max(0, speed || 0))
  if (state.samples.length > SAMPLE_COUNT) state.samples.shift()
}

function renderSpark() {
  const svg = $('spark')
  const samples = state.samples
  const peak = samples.reduce((max, v) => Math.max(max, v), 0)
  const scale = niceBytesCeil(peak * 1.08)
  svg.setAttribute('viewBox', `0 0 ${SAMPLE_COUNT} 100`)
  const offset = SAMPLE_COUNT - samples.length
  const bars = []
  samples.forEach((value, i) => {
    if (value <= 0) return
    const height = Math.max(2, (value / scale) * 100)
    const rect = document.createElementNS('http://www.w3.org/2000/svg', 'rect')
    rect.setAttribute('x', String(offset + i + 0.14))
    rect.setAttribute('y', String(100 - height))
    rect.setAttribute('width', '0.72')
    rect.setAttribute('height', String(height))
    rect.setAttribute('opacity', i === samples.length - 1 ? '1' : '0.5')
    bars.push(rect)
  })
  svg.replaceChildren(...bars)
  $('spark-scale').textContent = formatSpeed(scale, { trim: true })
}

/** Rows shown in the popup: transferring files, then packages processed remotely, verified or extracted. */
function activeItems() {
  const items = []
  for (const pkg of state.packages) {
    for (const file of pkg.files ?? []) {
      if (file.status === 'downloading' || file.status === 'resolving') items.push({ kind: 'file', file, pkg })
    }
  }
  for (const pkg of state.packages) {
    if (PROCESSING.includes(pkg.status)) items.push({ kind: 'remote', pkg })
  }
  return items
}

function buildRow() {
  const li = el('li', 'level-row')
  const body = el('div', 'row-body')
  const top = el('div', 'row-top')
  top.append(el('p', 'row-name truncate'), el('p', 'row-speed num'))
  body.append(top, el('p', 'row-meta num'))
  li.append(el('span', 'level-fill'), el('span', 'level-line'), body)
  return li
}

function fillRow(li, item) {
  const name = li.querySelector('.row-name')
  const speed = li.querySelector('.row-speed')
  const meta = li.querySelector('.row-meta')
  let ratio = null
  const parts = []

  if (item.kind === 'file') {
    const { file } = item
    const label = displayName(file)
    name.textContent = label
    name.title = label
    li.classList.remove('is-remote')
    if (file.status === 'resolving') {
      speed.textContent = ''
      parts.push(el('span', 'tone-info', t('rowResolving')))
      if (file.host) parts.push(el('span', 'faint', file.host))
    } else {
      ratio = file.size ? file.downloaded / file.size : null
      speed.textContent = formatSpeed(file.speed)
      parts.push(el('span', '', formatPercent(ratio)))
      const sizes = el('span', '', formatBytes(file.downloaded))
      if (file.size) sizes.append(el('span', 'faint', ` / ${formatBytes(file.size)}`))
      parts.push(sizes)
      parts.push(el('span', '', file.eta != null ? formatDuration(file.eta) : ''))
    }
  } else {
    const { pkg } = item
    name.textContent = pkg.name
    name.title = pkg.name
    li.classList.add('is-remote')
    const extracting = pkg.status === 'extracting'
    const verifying = pkg.status === 'verifying'
    ratio = extracting ? pkg.extract_progress : verifying ? pkg.verify_progress : (pkg.debrid?.progress ?? null)
    speed.textContent = formatPercent(ratio)
    const label = extracting ? t('rowExtracting') : verifying ? t('rowVerifying') : pkg.debrid?.status || t('rowRemote')
    parts.push(el('span', 'tone-info', label))
    if (pkg.status === 'debrid' && pkg.debrid?.speed) {
      parts.push(el('span', '', t('rowRemoteSpeed', { speed: formatSpeed(pkg.debrid.speed) })))
    }
  }

  meta.replaceChildren(...parts)
  li.classList.toggle('is-indeterminate', ratio == null)
  li.style.setProperty('--level', `${Math.min(100, Math.max(0, (ratio ?? 0) * 100))}%`)
}

function renderRows() {
  const list = $('rows')
  const items = activeItems()
  const existing = new Map([...list.children].map((li) => [li.dataset.key, li]))
  const ordered = items.map((item) => {
    const key = item.kind === 'file' ? `f${item.file.id}` : `p${item.pkg.id}`
    let li = existing.get(key)
    if (!li) {
      li = buildRow()
      li.dataset.key = key
    }
    existing.delete(key)
    fillRow(li, item)
    return li
  })
  for (const li of existing.values()) li.remove()
  ordered.forEach((li, index) => {
    if (list.children[index] !== li) list.insertBefore(li, list.children[index] ?? null)
  })

  $('rows-title').hidden = items.length === 0
  $('rows-count').textContent = String(items.length)
  const empty = items.length === 0
  $('empty').hidden = !empty
  if (empty) {
    const { waiting } = currentCounts()
    $('empty-body').textContent = waiting > 0 ? tn('emptyQueued', waiting) : t('emptyBody')
  }
}

function renderControls() {
  const paused = Boolean(state.system?.paused)
  $('toggle-pause-icon').replaceChildren(icon(paused ? 'play' : 'pause', { size: 15 }))
  $('toggle-pause-label').textContent = paused ? t('resumeAll') : t('pauseAll')
}

function renderAll() {
  renderMeter()
  renderRows()
  renderControls()
  showView('live')
}

// ---- Data ----------------------------------------------------------------------------------

async function fetchAll() {
  const [packages, system] = await Promise.all([state.client.packages(), state.client.system()])
  state.packages = packages
  state.system = system
}

let refetchTimer
function scheduleRefetch() {
  clearTimeout(refetchTimer)
  refetchTimer = setTimeout(async () => {
    try {
      state.packages = await state.client.packages()
      renderRows()
      renderMeter()
    } catch {
      // The stream or the poller will report a lasting problem.
    }
  }, REFETCH_DEBOUNCE_MS)
}

/** Merges a `progress` event into the packages; unknown ids mean the queue changed. */
function applyProgress({ downloads = [], packages = [] }) {
  const files = new Map()
  const pkgs = new Map()
  for (const pkg of state.packages) {
    pkgs.set(pkg.id, pkg)
    for (const file of pkg.files ?? []) files.set(file.id, file)
  }
  let unknown = false
  const seen = new Set()
  for (const live of downloads) {
    const file = files.get(live.id)
    if (!file) {
      unknown = true
      continue
    }
    seen.add(live.id)
    Object.assign(file, {
      status: live.status,
      downloaded: live.downloaded,
      size: live.size,
      speed: live.speed,
      eta: live.eta,
      connections: live.connections,
    })
  }
  // Files that left the live set stopped transferring (finished, paused, failed…).
  for (const file of files.values()) {
    if (!seen.has(file.id) && file.status === 'downloading') {
      file.speed = 0
      file.eta = null
      unknown = true
    }
  }
  for (const live of packages) {
    const pkg = pkgs.get(live.id)
    if (!pkg) {
      unknown = true
      continue
    }
    pkg.status = live.status
    pkg.debrid = live.debrid
    pkg.extract_progress = live.extract_progress
    pkg.verify_progress = live.verify_progress
  }
  if (unknown) scheduleRefetch()
}

// ---- Live updates: SSE first, polling as fallback ------------------------------------------

let stream = null
let watchdog
let pollTimer
let retryTimer

function armWatchdog() {
  clearTimeout(watchdog)
  watchdog = setTimeout(() => fallBackToPolling(), WATCHDOG_MS)
}

function onEvent(name, data) {
  armWatchdog()
  if (state.mode !== 'live') {
    state.mode = 'live'
    clearTimeout(pollTimer)
    clearTimeout(retryTimer)
  }
  switch (name) {
    case 'stats':
      state.stats = data
      state.system = { ...state.system, paused: data.paused, schedule_open: data.schedule_open }
      pushSample(data.speed)
      if (data.active === 0) {
        for (const pkg of state.packages) for (const file of pkg.files ?? []) file.speed = 0
      }
      renderAll()
      break
    case 'progress':
      applyProgress(data)
      renderRows()
      break
    case 'queue':
      scheduleRefetch()
      break
    case 'resync':
      fetchAll().then(renderAll, () => {})
      break
    default:
      break
  }
  setConnection(true)
}

function startStream() {
  stream?.abort()
  const controller = new AbortController()
  stream = controller
  armWatchdog()
  state.client.events({ signal: controller.signal, onEvent }).catch((error) => {
    if (stream !== controller) return // replaced or stopped on purpose
    stream = null
    // An authentication problem won't be fixed by polling.
    if (error instanceof ApiError && needsSettings(error)) {
      clearTimeout(watchdog)
      showError(error)
      return
    }
    fallBackToPolling()
  })
}

function fallBackToPolling({ immediately = true } = {}) {
  clearTimeout(watchdog)
  const current = stream
  stream = null
  current?.abort()
  if (state.mode === 'polling') return
  state.mode = 'polling'
  state.stats = null
  setConnection(state.connected)
  if (immediately) poll()
  else pollTimer = setTimeout(poll, POLL_MS)
  clearTimeout(retryTimer)
  retryTimer = setTimeout(function retry() {
    if (state.mode !== 'polling') return
    startStream()
    retryTimer = setTimeout(retry, STREAM_RETRY_MS)
  }, STREAM_RETRY_MS)
}

async function poll() {
  clearTimeout(pollTimer)
  if (state.mode !== 'polling') return
  try {
    await fetchAll()
    pushSample(summarize(state.packages).speed)
    setConnection(true)
    renderAll()
  } catch (error) {
    showError(error)
    if (needsSettings(error)) return
  }
  if (state.mode === 'polling') pollTimer = setTimeout(poll, POLL_MS)
}

function stopLive() {
  clearTimeout(watchdog)
  clearTimeout(pollTimer)
  clearTimeout(retryTimer)
  const current = stream
  stream = null
  current?.abort()
  state.mode = 'idle'
}

// ---- Actions -------------------------------------------------------------------------------

function openSettings() {
  ext.runtime.openOptionsPage()
  window.close()
}

async function togglePause() {
  const button = $('toggle-pause')
  button.disabled = true
  try {
    const system = state.system?.paused ? await state.client.resume() : await state.client.pause()
    if (system) state.system = { ...state.system, ...system }
    renderControls()
    renderMeter()
  } catch (error) {
    showFeedback('bad', errorText(error, state.settings.serverUrl))
  } finally {
    button.disabled = false
  }
}

let feedbackTimer
function showFeedback(tone, text) {
  fillFeedback($('add-feedback'), tone, text)
  clearTimeout(feedbackTimer)
  if (tone === 'ok') feedbackTimer = setTimeout(() => (box.hidden = true), 4000)
}

function parsedInput() {
  const { links, magnets } = parseLinks($('links').value)
  return { links, magnets, all: [...links, ...magnets] }
}

/** True while links are being sent: one send at a time. */
let sending = false

function renderDetected() {
  const { links, magnets, all } = parsedInput()
  const detected = $('detected')
  const parts = []
  if (links.length) parts.push(tn('detectedLinks', links.length))
  if (magnets.length) parts.push(tn('detectedMagnets', magnets.length))
  const hasText = $('links').value.trim().length > 0
  detected.textContent = parts.length ? parts.join(' · ') : hasText ? t('detectedNone') : ''
  detected.classList.toggle('has-links', all.length > 0)
  $('add-btn').disabled = sending || all.length === 0
}

async function submitLinks(event) {
  event.preventDefault()
  if (sending) return
  const { all } = parsedInput()
  if (!all.length) return
  sending = true
  const input = $('links')
  input.readOnly = true
  try {
    const { packages, notice } = await withSpinner($('add-btn'), () =>
      sendLinks(all, { paused: $('add-paused').checked }),
    )
    input.value = ''
    if (notice) showFeedback('warn', notice)
    else showFeedback('ok', tn('sentPackages', packages.length || 1))
    ext.runtime.sendMessage({ type: 'sent', count: all.length }).catch(() => {})
    scheduleRefetch()
  } catch (error) {
    showFeedback(error?.maybeAdded ? 'warn' : 'bad', errorText(error, state.settings.serverUrl))
  } finally {
    sending = false
    input.readOnly = false
    renderDetected()
  }
}

async function scanPage() {
  const button = $('page-links')
  button.disabled = true
  try {
    const [tab] = await ext.tabs.query({ active: true, currentWindow: true })
    const result = tab?.id != null ? await ext.runtime.sendMessage({ type: 'open-picker', tabId: tab.id }) : null
    if (result?.ok) window.close()
    else showFeedback('bad', t('pageLinksUnavailable'))
  } catch {
    showFeedback('bad', t('pageLinksUnavailable'))
  } finally {
    button.disabled = false
  }
}

// ---- Start ---------------------------------------------------------------------------------

async function start() {
  stopLive()
  state.settings = await loadSettings()
  const { serverUrl } = state.settings
  if (!serverUrl) {
    showError(new ApiError('unconfigured'))
    return
  }
  $('conn').hidden = false
  $('conn-host').textContent = displayHost(serverUrl)
  $('open-app').href = serverUrl
  $('add-paused').checked = state.settings.addPaused

  if (!(await hasHostPermission(serverUrl))) {
    showError(new ApiError('permission'))
    return
  }

  state.client = createClient(state.settings)
  // Only show "Connecting…" if the server is slow to answer, to avoid a flash.
  const slow = setTimeout(
    () =>
      showMessage({
        spinner: true,
        title: t('connectingTitle'),
        body: displayHost(serverUrl),
        secondary: [t('openSettings'), openSettings],
      }),
    300,
  )
  try {
    await fetchAll()
  } catch (error) {
    clearTimeout(slow)
    showError(error)
    // Keep trying while the popup is open: the view comes back by itself.
    if (!needsSettings(error)) fallBackToPolling({ immediately: false })
    return
  }
  clearTimeout(slow)
  setConnection(true)
  pushSample(summarize(state.packages).speed)
  renderAll()
  state.mode = 'connecting'
  startStream()
}

localize()
hydrateIcons()
$('open-settings').addEventListener('click', openSettings)
$('open-app').addEventListener('click', (event) => {
  event.preventDefault()
  if (state.settings?.serverUrl) ext.tabs.create({ url: state.settings.serverUrl })
  window.close()
})
$('toggle-pause').addEventListener('click', togglePause)
$('page-links').addEventListener('click', scanPage)
$('add-form').addEventListener('submit', submitLinks)
$('links').addEventListener('input', renderDetected)
$('links').addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) $('add-form').requestSubmit()
})
window.addEventListener('pagehide', stopLive)
start()

// Read-only status for automated tests.
globalThis.rustloadPopup = {
  get mode() {
    return state.mode
  },
  get paused() {
    return Boolean(state.system?.paused)
  },
}
