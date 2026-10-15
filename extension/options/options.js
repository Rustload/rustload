import { createClient } from '../lib/api.js'
import { ext } from '../lib/browser.js'
import { $, categoryOptions, el, iconSlot } from '../lib/dom.js'
import { errorText } from '../lib/errors.js'
import { formatBytes } from '../lib/format.js'
import { localize, t } from '../lib/i18n.js'
import { hydrateIcons } from '../lib/icons.js'
import {
  hasHostPermission,
  loadSettings,
  originPattern,
  parseServerUrl,
  saveSettings,
} from '../lib/settings.js'

/** Settings as last saved. */
let saved = null
/** Categories loaded from the server, or null while unknown. */
let categories = null
/** Server address `categories` come from (it may be a tested address not saved yet). */
let categoriesFrom = ''
/** Increments on every test so that a slow, older answer never overwrites a newer one. */
let testRun = 0

// ---- Host permission -----------------------------------------------------------------------

/**
 * Asks the browser for access to the server's origin (and nothing else). Must be called
 * synchronously from the click handler: Firefox drops the user gesture after an `await`.
 */
function requestAccess(serverUrl) {
  try {
    return ext.permissions.request({ origins: [originPattern(serverUrl)] }).catch(() => false)
  } catch {
    return Promise.resolve(false)
  }
}

async function renderPermission() {
  const box = $('perm-host-status')
  box.replaceChildren()
  if (!saved.serverUrl) {
    box.append(el('span', 'faint', t('permNoServer')))
    return
  }
  const host = new URL(saved.serverUrl).hostname
  if (await hasHostPermission(saved.serverUrl)) {
    box.append(iconSlot('check', { className: 'tone-ok' }), el('span', '', t('permGranted', { host })))
    return
  }
  const grant = el('button', 'btn btn-secondary btn-sm', t('permGrant'))
  grant.type = 'button'
  grant.addEventListener('click', async () => {
    const granted = await requestAccess(saved.serverUrl)
    await renderPermission()
    if (granted) runTest()
  })
  box.append(el('span', '', t('permNotGranted')), grant)
}

// ---- Form state ----------------------------------------------------------------------------

function selectedCategory() {
  const select = $('category')
  const value = select.value
  if (!value) return { categoryId: null, categoryName: '' }
  const known = categories?.find((c) => String(c.id) === value)
  return { categoryId: Number(value), categoryName: known?.name ?? saved.categoryName }
}

function formValues() {
  return {
    serverUrlText: $('server-url').value.trim(),
    apiKey: $('api-key').value.trim(),
    addPaused: $('add-paused').checked,
    notify: $('notify').checked,
    ...selectedCategory(),
  }
}

function isDirty() {
  const values = formValues()
  const parsed = parseServerUrl(values.serverUrlText)
  const url = parsed.url ?? values.serverUrlText
  return (
    url !== saved.serverUrl ||
    values.apiKey !== saved.apiKey ||
    values.addPaused !== saved.addPaused ||
    values.notify !== saved.notify ||
    values.categoryId !== saved.categoryId
  )
}

let statusTimer
function setSaveStatus(kind) {
  const box = $('save-status')
  clearTimeout(statusTimer)
  box.replaceChildren()
  if (kind === 'saved') {
    box.append(iconSlot('check', { className: 'tone-ok' }), el('span', '', t('optSaved')))
    statusTimer = setTimeout(() => setSaveStatus(isDirty() ? 'dirty' : ''), 3000)
  } else if (kind === 'dirty') {
    const dot = el('span', 'status-dot is-warn')
    box.append(dot, el('span', '', t('optUnsaved')))
  }
}

function onFormChange() {
  setSaveStatus(isDirty() ? 'dirty' : '')
}

function urlErrorText(error) {
  return {
    invalid: t('errInvalidUrl'),
    scheme: t('errUrlScheme'),
    credentials: t('errUrlCredentials'),
    empty: t('errInvalidUrl'),
  }[error]
}

/** The error replaces the hint (like the web UI's fields), so the layout doesn't jump. */
function showUrlError(error) {
  const hint = $('server-url-hint')
  const input = $('server-url')
  hint.classList.toggle('field-error', Boolean(error))
  hint.classList.toggle('hint', !error)
  hint.textContent = error ? urlErrorText(error) : t('optServerUrlHint')
  if (error) input.setAttribute('aria-invalid', 'true')
  else input.removeAttribute('aria-invalid')
}

// ---- Categories ----------------------------------------------------------------------------

function renderCategories() {
  const select = $('category')
  const current = saved.categoryId
  const options = categoryOptions(categories, saved, (name) => t('optCategoryMissing', { name }))
  const previous = select.value
  select.replaceChildren(...options)
  select.value = previous && options.some((o) => o.value === previous) ? previous : current == null ? '' : String(current)
  $('category-hint').textContent = categories ? t('optCategoryHint') : t('optCategoryOffline')
}

async function loadCategories({ serverUrl, apiKey }) {
  try {
    categories = await createClient({ serverUrl, apiKey }).categories()
  } catch {
    categories = null
  }
  categoriesFrom = serverUrl
  renderCategories()
}

// ---- Connection test -----------------------------------------------------------------------

function showTestResult(kind, content) {
  const box = $('test-result')
  box.replaceChildren()
  if (kind === 'pending') {
    box.append(el('span', 'spinner'), el('span', '', content))
  } else if (kind === 'ok') {
    box.append(iconSlot('circle-check', { className: 'tone-ok' }), content)
  } else if (kind === 'bad') {
    box.append(iconSlot('circle-alert', { className: 'tone-bad' }), el('span', 'tone-bad', content))
  }
}

async function runTest({ serverUrl = saved.serverUrl, apiKey = saved.apiKey, granted = Promise.resolve(true) } = {}) {
  const run = ++testRun
  showTestResult('pending', t('optTesting'))
  $('test').disabled = true
  try {
    if (!(await granted)) {
      showTestResult('bad', errorText({ kind: 'permission' }, serverUrl))
      return
    }
    const client = createClient({ serverUrl, apiKey })
    const system = await client.system()
    if (run !== testRun) return
    const text = el('span')
    text.append(el('strong', '', t('optConnected', { version: system.version })))
    if (system.disk?.free != null) {
      text.append(el('span', 'num test-detail', t('optDiskFree', { size: formatBytes(system.disk.free) })))
    }
    showTestResult('ok', text)
    await loadCategories({ serverUrl, apiKey })
  } catch (error) {
    if (run !== testRun) return
    showTestResult('bad', errorText(error, serverUrl))
  } finally {
    if (run === testRun) $('test').disabled = false
    renderPermission()
  }
}

// ---- Save ----------------------------------------------------------------------------------

async function save(serverUrl, granted) {
  // Wait for the browser's permission prompt to be answered before testing.
  const access = await granted
  const values = formValues()
  const previous = saved.serverUrl
  const next = {
    serverUrl,
    apiKey: values.apiKey,
    addPaused: values.addPaused,
    notify: values.notify,
    categoryId: values.categoryId,
    categoryName: values.categoryName,
  }
  // A different server makes the category meaningless, unless it was chosen from that server's
  // list (loaded by a connection test before saving).
  if (previous && serverUrl !== previous) {
    if (categoriesFrom !== serverUrl) categories = null
    if (!categories?.some((c) => c.id === next.categoryId)) {
      next.categoryId = null
      next.categoryName = ''
    }
  }
  await saveSettings(next)
  saved = { ...saved, ...next }
  $('server-url').value = serverUrl
  renderCategories()
  setSaveStatus('saved')

  // Give back access to a server that is no longer used.
  if (previous && previous !== serverUrl && (!serverUrl || originPattern(previous) !== originPattern(serverUrl))) {
    ext.permissions.remove({ origins: [originPattern(previous)] }).catch(() => {})
  }
  await renderPermission()
  if (serverUrl) await runTest({ granted: Promise.resolve(access) })
  else showTestResult('', '')
}

// ---- Init ----------------------------------------------------------------------------------

async function init() {
  localize()
  hydrateIcons()
  $('version').textContent = `v${ext.runtime.getManifest().version}`

  saved = await loadSettings()
  $('server-url').value = saved.serverUrl
  $('api-key').value = saved.apiKey
  $('add-paused').checked = saved.addPaused
  $('notify').checked = saved.notify
  renderCategories()
  await renderPermission()

  const form = $('form')
  form.addEventListener('input', onFormChange)
  form.addEventListener('change', onFormChange)

  $('server-url').addEventListener('input', () => {
    if ($('server-url').getAttribute('aria-invalid')) showUrlError(null)
  })

  $('toggle-key').addEventListener('click', () => {
    const input = $('api-key')
    const show = input.type === 'password'
    input.type = show ? 'text' : 'password'
    const button = $('toggle-key')
    button.setAttribute('aria-pressed', String(show))
    button.title = t(show ? 'optHideKey' : 'optShowKey')
    button.setAttribute('aria-label', button.title)
    button.replaceChildren(iconSlot(show ? 'eye-off' : 'eye'))
  })

  $('test').addEventListener('click', () => {
    const parsed = parseServerUrl($('server-url').value)
    if (parsed.error) {
      showUrlError(parsed.error)
      showTestResult('bad', urlErrorText(parsed.error))
      return
    }
    const granted = requestAccess(parsed.url)
    runTest({ serverUrl: parsed.url, apiKey: $('api-key').value.trim(), granted })
  })

  $('reload-categories').addEventListener('click', () => {
    const parsed = parseServerUrl($('server-url').value)
    if (parsed.error) return
    const apiKey = $('api-key').value.trim()
    requestAccess(parsed.url).then((granted) => {
      if (granted) loadCategories({ serverUrl: parsed.url, apiKey })
    })
  })

  form.addEventListener('submit', (event) => {
    event.preventDefault()
    const parsed = parseServerUrl($('server-url').value)
    if (parsed.error && parsed.error !== 'empty') {
      showUrlError(parsed.error)
      $('server-url').focus()
      return
    }
    showUrlError(null)
    const serverUrl = parsed.url ?? ''
    const granted = serverUrl ? requestAccess(serverUrl) : Promise.resolve(true)
    save(serverUrl, granted)
  })

  if (saved.serverUrl && (await hasHostPermission(saved.serverUrl))) runTest()
  else if (!saved.serverUrl) $('server-url').focus()
}

init()
