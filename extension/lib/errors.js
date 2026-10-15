import { t } from './i18n.js'
import { displayHost } from './settings.js'

/** One precise sentence for an `ApiError` (or anything thrown), ready to show to the user. */
export function errorText(error, serverUrl = '') {
  const url = serverUrl || '?'
  switch (error?.kind) {
    case 'unconfigured':
      return t('errUnconfigured')
    case 'permission':
      return t('errPermission', { host: displayHost(serverUrl) })
    case 'timeout':
      if (error.maybeAdded) return t('errAddTimeout', { seconds: error.seconds })
      return t('errTimeout', { url, seconds: error.seconds || 8 })
    case 'unreachable':
      return t('errUnreachable', { url })
    case 'auth':
      return t(error.missingKey ? 'errAuthMissing' : 'errAuth')
    case 'refused':
      return t('errRefused')
    case 'notRustload':
      return t('errNotRustload', { url })
    case 'server':
      return t('errServer', { status: error.status })
    case 'rejected':
      if (error.code === 'NO_LINKS') return t('errNoLinks')
      if (error.code === 'NO_MAGNET_ACCOUNT') return t('errNoMagnetAccount')
      return t('errRejected', { message: error.detail || error.code })
    default:
      return error?.message || String(error)
  }
}

/** Title + icon for the full-size message states (popup, picker). */
export function errorHeadline(error) {
  switch (error?.kind) {
    case 'unconfigured':
      return { title: t('setupTitle'), icon: 'plug' }
    case 'permission':
      return { title: t('permissionTitle'), icon: 'shield-check' }
    case 'timeout':
    case 'unreachable':
      return { title: t('unreachableTitle'), icon: 'wifi-off' }
    case 'auth':
      return { title: t('authTitle'), icon: 'key-round' }
    case 'refused':
      return { title: t('refusedTitle'), icon: 'key-round' }
    case 'notRustload':
      return { title: t('notRustloadTitle'), icon: 'server' }
    default:
      return { title: t('serverErrorTitle'), icon: 'circle-alert' }
  }
}

/** Title of a failed send: after a timeout, the links may have been added anyway. */
export function sendFailedTitle(error) {
  return t(error?.maybeAdded ? 'notifyUncertainTitle' : 'notifyFailedTitle')
}

/** Errors only the settings page can fix. */
export function needsSettings(error) {
  return ['unconfigured', 'permission', 'auth', 'refused', 'notRustload'].includes(error?.kind)
}
