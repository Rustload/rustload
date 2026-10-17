import { t } from '$lib/i18n/index.svelte'

/**
 * Set by the desktop app on its window: `local` when Rustload runs inside the app, `gateway` when
 * the window shows a Rustload server. Undefined in a browser.
 */
export const appWindow = (window as Window & { rustloadApp?: 'local' | 'gateway' }).rustloadApp

export const isWindows = /^Win/i.test(navigator.platform)
const isMac = /^Mac/i.test(navigator.platform)

/** Where the desktop app's icon is: the notification area, or the menu bar on macOS. */
export function iconPlace(): string {
  return t(isMac ? 'app.placeMac' : 'app.place')
}
