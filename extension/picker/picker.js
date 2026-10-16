import { createClient } from '../lib/api.js'
import { checkInBatches, checkSummary, checkTone } from '../lib/check.js'
import { ext } from '../lib/browser.js'
import { $, categoryOptions, el, iconSlot, showFeedback, withSpinner } from '../lib/dom.js'
import { errorText, needsSettings } from '../lib/errors.js'
import { formatBytes } from '../lib/format.js'
import { localize, t, tn } from '../lib/i18n.js'
import { hydrateIcons, icon } from '../lib/icons.js'
import { describeLink } from '../lib/links.js'
import { sendLinks } from '../lib/send.js'
import { hasHostPermission, loadSettings } from '../lib/settings.js'

const MAX_CHIPS = 8

const state = {
  settings: null,
  page: null,
  items: [],
  /** 'files' (links that look like downloads) | 'all' */
  mode: 'files',
  query: '',
  /** Facet chip: an extension, 'magnet', 'hoster' or 'media'; null = any. */
  facet: null,
  host: '',
  selected: new Set(),
  /** The category select is filled (until then, sends use the saved default). */
  categoriesReady: false,
  /** Set while sending, and for good once sent (the window is closing). */
  sending: false,
  /** Links checked by the server: url → result (lib/check.js); `checking` holds those asked. */
  checks: new Map(),
  checking: new Set(),
  /** Dead links taken out of the selection by the check. */
  unticked: 0,
  checkError: '',
}

const isFileLike = (item) => item.kind !== 'page'

function facetOf(item) {
  if (item.kind === 'magnet' || item.kind === 'hoster' || item.kind === 'media') return item.kind
  return item.ext || null
}

function facetLabel(facet) {
  if (facet === 'magnet') return t('facetMagnets')
  if (facet === 'hoster') return t('facetHosters')
  if (facet === 'media') return t('facetMedia')
  if (facet === 'split') return t('facetSplit')
  return `.${facet}`
}

// ---- Filtering -----------------------------------------------------------------------------

function inMode(item) {
  return state.mode === 'all' || isFileLike(item)
}

function matchesQuery(item) {
  const words = state.query.toLowerCase().split(/\s+/).filter(Boolean)
  if (!words.length) return true
  const haystack = `${item.url} ${item.name} ${item.text} ${item.host}`.toLowerCase()
  return words.every((word) => haystack.includes(word))
}

function visibleItems() {
  return state.items.filter(
    (item) =>
      inMode(item) &&
      (!state.facet || facetOf(item) === state.facet) &&
      (!state.host || item.host === state.host) &&
      matchesQuery(item),
  )
}

function countBy(items, keyOf) {
  const counts = new Map()
  for (const item of items) {
    const key = keyOf(item)
    if (key) counts.set(key, (counts.get(key) ?? 0) + 1)
  }
  return [...counts.entries()].sort((a, b) => b[1] - a[1] || String(a[0]).localeCompare(String(b[0])))
}

// ---- Rendering -----------------------------------------------------------------------------

function renderModes() {
  const files = state.items.filter(isFileLike).length
  $('count-files').textContent = String(files)
  $('count-all').textContent = String(state.items.length)
  for (const button of document.querySelectorAll('.segment')) {
    button.setAttribute('aria-checked', String(button.dataset.mode === state.mode))
  }
}

function renderFacets() {
  const pool = state.items.filter(inMode)
  const facets = countBy(pool, facetOf).slice(0, MAX_CHIPS)
  if (state.facet && !facets.some(([facet]) => facet === state.facet)) state.facet = null
  const chips = facets.map(([facet, count]) => {
    const chip = el('button', 'chip')
    chip.type = 'button'
    chip.setAttribute('aria-pressed', String(state.facet === facet))
    chip.dataset.facet = facet
    chip.append(el('span', '', facetLabel(facet)), el('span', 'num', String(count)))
    chip.addEventListener('click', () => {
      state.facet = state.facet === facet ? null : facet
      render()
      // The chips were rebuilt: keep the keyboard focus on this one.
      $('chips').querySelector(`[data-facet="${CSS.escape(facet)}"]`)?.focus()
    })
    return chip
  })
  $('chips').replaceChildren(...chips)

  const hosts = countBy(pool, (item) => item.host)
  if (state.host && !hosts.some(([host]) => host === state.host)) state.host = ''
  const select = $('host')
  select.replaceChildren(
    new Option(t('pickerAllHosts'), ''),
    ...hosts.map(([host, count]) => new Option(`${host} (${count})`, host)),
  )
  select.value = state.host
  $('facets').hidden = chips.length === 0 && hosts.length <= 1
}

function buildRow(item) {
  const li = el('li')
  const label = el('label', 'pick-row')
  label.title = item.url
  const box = el('input', 'box')
  box.type = 'checkbox'
  box.checked = state.selected.has(item.url)
  box.addEventListener('change', () => {
    if (box.checked) state.selected.add(item.url)
    else state.selected.delete(item.url)
    renderSelection()
  })
  const result = state.checks.get(item.url)
  const asked = state.checking.has(item.url) || Boolean(result)
  const main = el('span', 'pick-main')
  const name = el('span', 'pick-name')
  if (asked) {
    const dot = el('span', 'check-dot')
    dot.dataset.tone = checkTone(result)
    name.append(dot)
  }
  // The host's name for the file (a file host's link rarely tells it).
  name.append(el('span', '', result?.media?.title || result?.filename || item.name))
  main.append(name)
  const meta = el('span', 'pick-meta')
  if (item.host) meta.append(el('span', 'host', item.host))
  if (asked) {
    const dead = checkTone(result) === 'bad'
    meta.append(el('span', dead ? 'pick-check is-dead' : 'pick-check', checkSummary(result, { size: false })))
  }
  const text = item.text && item.text !== item.name ? item.text : ''
  if (text) meta.append(el('span', '', text))
  if (!item.host && !text && !asked) meta.append(document.createTextNode(item.url.slice(0, 120)))
  main.append(meta)
  const kind = el('span', 'pick-kind num')
  if (result?.size) {
    kind.classList.add('is-size')
    kind.textContent = formatBytes(result.size)
  }
  else if (item.kind === 'magnet') kind.append(iconSlot('magnet', { size: 13 }))
  else if (item.ext && item.ext !== 'split') kind.textContent = item.ext
  label.append(box, main, kind)
  li.append(label)
  return li
}

function renderSelection() {
  const visible = visibleItems()
  const chosen = visible.filter((item) => state.selected.has(item.url))
  $('selection-count').textContent = t('pickerSelected', { count: chosen.length, total: visible.length })
  const all = $('select-all')
  all.checked = visible.length > 0 && chosen.length === visible.length
  all.indeterminate = chosen.length > 0 && chosen.length < visible.length
  all.disabled = visible.length === 0
  // While sending, the button holds a spinner (its label is detached) and stays disabled.
  if (!state.sending) {
    $('send-label').textContent = chosen.length ? tn('pickerSend', chosen.length) : t('pickerSendNone')
    $('send').disabled = chosen.length === 0
  }
  return chosen
}

function showListState({ iconName, title, body = '', actions = [] }) {
  $('state-icon').replaceChildren(icon(iconName, { size: 20, strokeWidth: 1.75 }))
  $('state-title').textContent = title
  $('state-body').textContent = body
  $('state-body').hidden = !body
  $('state-actions').replaceChildren(...actions)
  $('list-state').hidden = false
}

function render() {
  renderModes()
  renderFacets()
  const visible = visibleItems()
  $('list').replaceChildren(...visible.map(buildRow))
  $('list-state').hidden = true
  if (!visible.length) {
    showListState({ iconName: 'search', title: t('pickerNoMatch') })
  }
  renderCheckState()
  renderSelection()
}

// ---- Checking ------------------------------------------------------------------------------

function renderCheckState() {
  const box = $('check-state')
  box.className = 'check-state truncate'
  box.title = ''
  const pending = [...state.checking].some((url) => !state.checks.has(url))
  if (pending) {
    box.replaceChildren(el('span', 'spinner'), el('span', '', t('pickerChecking')))
  } else if (state.checkError) {
    box.classList.add('faint')
    box.textContent = t('pickerCheckFailed')
    box.title = state.checkError
  } else if (state.unticked) {
    box.classList.add('tone-bad')
    box.textContent = tn('pickerUnticked', state.unticked)
  } else {
    box.hidden = true
    return
  }
  box.hidden = false
}

/**
 * Has the server check the links that look like downloads: whether they are online, their real
 * name and size, and who will download them. Dead links leave the selection.
 */
async function checkLinks() {
  const urls = state.items.filter(isFileLike).map((item) => item.url)
  if (!urls.length) return
  for (const url of urls) state.checking.add(url)
  render()
  try {
    await checkInBatches(createClient(state.settings), urls, (results) => {
      for (const result of results) {
        state.checks.set(result.url, result)
        const dead = result.status === 'offline' || result.status === 'unsupported'
        if (dead && !state.sending && state.selected.delete(result.url)) state.unticked += 1
      }
      if (!state.sending) render()
    })
  } catch (error) {
    state.checkError = errorText(error, state.settings.serverUrl)
    // Links left unchecked show no status rather than a check that never ends.
    for (const url of state.checking) if (!state.checks.has(url)) state.checking.delete(url)
    if (!state.sending) render()
  }
}

// ---- Sending -------------------------------------------------------------------------------

function settingsButton() {
  const button = el('button', 'btn btn-secondary btn-sm', t('openSettings'))
  button.type = 'button'
  button.addEventListener('click', () => ext.runtime.openOptionsPage())
  return button
}

function showError(text, { withSettings = false, tone = 'bad' } = {}) {
  const box = $('feedback')
  showFeedback(box, tone, text)
  if (withSettings) {
    const action = settingsButton()
    action.classList.add('feedback-action')
    box.append(action)
  }
}

async function closeWindow() {
  try {
    const current = await ext.windows.getCurrent()
    await ext.windows.remove(current.id)
  } catch {
    window.close()
  }
}

function closeButton() {
  const button = el('button', 'btn btn-secondary btn-sm', t('close'))
  button.type = 'button'
  button.addEventListener('click', closeWindow)
  return button
}

async function send() {
  if (state.sending) return
  const chosen = renderSelection()
  if (!chosen.length) return
  state.sending = true
  $('feedback').hidden = true
  const value = $('category').value
  const urls = chosen.map((item) => item.url)
  try {
    const { packages, notice } = await withSpinner($('send'), () =>
      sendLinks(urls, {
        paused: $('paused').checked,
        // Until the server's list is shown, keep the saved default category.
        categoryId: state.categoriesReady ? (value ? Number(value) : null) : undefined,
      }),
    )
    ext.runtime.sendMessage({ type: 'sent', count: urls.length }).catch(() => {})
    for (const id of ['tools', 'facets', 'list-head', 'footer']) $(id).hidden = true
    document.body.classList.add('is-final')
    $('list').replaceChildren()
    const names = packages.map((pkg) => pkg.name).filter(Boolean)
    showListState({
      iconName: 'circle-check',
      title: tn('sentLinks', urls.length),
      body: notice || (names.length > 1 ? tn('morePackages', names.length - 1, { name: names[0] }) : (names[0] ?? '')),
      // A notice stays until it is read.
      actions: notice ? [closeButton()] : [],
    })
    $('state-icon').classList.add('tone-ok')
    if (!notice) setTimeout(closeWindow, 1600)
  } catch (error) {
    showError(errorText(error, state.settings.serverUrl), {
      withSettings: needsSettings(error),
      tone: error?.maybeAdded ? 'warn' : 'bad',
    })
    state.sending = false
    renderSelection()
  }
}

// ---- Categories ----------------------------------------------------------------------------

async function renderCategories() {
  const select = $('category')
  const { categoryId } = state.settings
  let list = null
  if (state.settings.serverUrl && (await hasHostPermission(state.settings.serverUrl))) {
    try {
      list = await createClient(state.settings).categories()
    } catch {
      list = null
    }
  }
  select.replaceChildren(...categoryOptions(list, state.settings))
  select.value = categoryId == null ? '' : String(categoryId)
  state.categoriesReady = true
}

// ---- Start ---------------------------------------------------------------------------------

async function main() {
  localize()
  hydrateIcons()
  state.settings = await loadSettings()

  const id = new URLSearchParams(location.search).get('id') ?? ''
  const key = `pick:${id}`
  const page = (await ext.storage.session.get(key))[key]
  if (!page) {
    document.body.classList.add('is-final')
    showListState({ iconName: 'list-checks', title: t('pickerExpiredTitle'), body: t('pickerExpiredBody') })
    return
  }
  state.page = page
  let host = ''
  try {
    host = new URL(page.url).host
  } catch {
    host = ''
  }
  $('page').textContent = page.title ? `${page.title} — ${host}` : host
  $('page').title = page.url
  document.title = `${t('pickerTitle')} — ${page.title || host}`

  const seen = new Set()
  state.items = page.links
    .map((link) => ({ ...describeLink(link.url), text: link.text ?? '' }))
    .filter((item) => !seen.has(item.url) && seen.add(item.url))

  if (!state.items.length) {
    document.body.classList.add('is-final')
    showListState({ iconName: 'link', title: t('pickerEmptyTitle'), body: t('pickerEmptyBody') })
    return
  }

  const files = state.items.filter(isFileLike)
  state.mode = files.length ? 'files' : 'all'
  // Channels and playlists of media sites are listed but left unticked: they can be huge.
  for (const item of files) if (item.single !== false) state.selected.add(item.url)

  $('tools').hidden = false
  $('list-head').hidden = false
  $('footer').hidden = false
  $('paused').checked = state.settings.addPaused

  for (const button of document.querySelectorAll('.segment')) {
    button.addEventListener('click', () => {
      state.mode = button.dataset.mode
      render()
    })
  }
  $('filter').addEventListener('input', (event) => {
    state.query = event.target.value
    render()
  })
  $('host').addEventListener('change', (event) => {
    state.host = event.target.value
    render()
  })
  $('select-all').addEventListener('change', (event) => {
    for (const item of visibleItems()) {
      if (event.target.checked) state.selected.add(item.url)
      else state.selected.delete(item.url)
    }
    render()
  })
  $('send').addEventListener('click', send)
  document.addEventListener('keydown', (event) => {
    // Keys confirming an input method's composition (CJK…) belong to the input method.
    if (event.isComposing) return
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) send()
    if (event.key !== 'Escape') return
    const filter = $('filter')
    // Escape first clears the filter, then closes the window.
    if (event.target === filter && filter.value) {
      event.preventDefault()
      filter.value = ''
      state.query = ''
      render()
      return
    }
    closeWindow()
  })

  render()

  if (!state.settings.serverUrl) showError(t('errUnconfigured'), { withSettings: true })
  else if (!(await hasHostPermission(state.settings.serverUrl))) {
    showError(errorText({ kind: 'permission' }, state.settings.serverUrl), { withSettings: true })
  } else checkLinks()
  renderCategories()
}

main()
