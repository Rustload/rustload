/*
 * Firefox `protocol_handlers` target: magnet links clicked anywhere open this page with the
 * magnet in `?uri=`, and it is sent straight to Rustload.
 */
import { ext } from '../lib/browser.js'
import { $ } from '../lib/dom.js'
import { errorText, needsSettings, sendFailedTitle } from '../lib/errors.js'
import { localize, t } from '../lib/i18n.js'
import { icon } from '../lib/icons.js'
import { MAGNET_RE, describeLink } from '../lib/links.js'
import { sendLinks } from '../lib/send.js'
import { loadSettings } from '../lib/settings.js'

async function closeTab() {
  try {
    const tab = await ext.tabs.getCurrent()
    await ext.tabs.remove(tab.id)
  } catch {
    window.close()
  }
}

function finish(iconName, tone, title, body = '') {
  $('icon').replaceChildren(icon(iconName, { size: 20, strokeWidth: 1.75 }))
  $('icon').className = `state-icon ${tone}`
  $('title').textContent = title
  $('body').textContent = body
  $('actions').hidden = false
}

async function main() {
  localize()
  $('close').addEventListener('click', closeTab)
  $('settings').addEventListener('click', () => ext.runtime.openOptionsPage())

  const uri = new URLSearchParams(location.search).get('uri') ?? ''
  if (!MAGNET_RE.test(uri)) {
    finish('circle-alert', 'tone-bad', t('magnetInvalid'))
    return
  }
  $('name').textContent = describeLink(uri).name
  $('name').title = uri

  try {
    const { notice } = await sendLinks([uri])
    ext.runtime.sendMessage({ type: 'sent', count: 1 }).catch(() => {})
    // A notice stays until it is read.
    finish('circle-check', 'tone-ok', t('magnetAdded'), notice || t('magnetClosing'))
    if (!notice) setTimeout(closeTab, 2500)
  } catch (error) {
    const { serverUrl } = await loadSettings()
    finish('circle-alert', 'tone-bad', sendFailedTitle(error), errorText(error, serverUrl))
    $('settings').hidden = !needsSettings(error)
  }
}

main()
