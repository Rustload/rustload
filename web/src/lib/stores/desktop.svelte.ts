import { readStored, writeStored } from '$lib/utils/storage'

const STORAGE_KEY = 'rustload.desktopNotifications'

/**
 * Browser notifications for finished and failed packages while the tab is in the background.
 * A per-browser preference; browsers only allow them on HTTPS or localhost.
 */
class DesktopNotifications {
  readonly supported = 'Notification' in window && window.isSecureContext
  permission = $state<NotificationPermission>(this.supported ? Notification.permission : 'denied')
  enabled = $state(this.supported && Notification.permission === 'granted' && readStored(STORAGE_KEY) === '1')

  async enable(): Promise<void> {
    if (!this.supported) return
    this.permission = await Notification.requestPermission()
    this.#save(this.permission === 'granted')
  }

  disable(): void {
    this.#save(false)
  }

  show(title: string, body: string, tag: string): void {
    if (!this.enabled || this.permission !== 'granted' || !document.hidden) return
    try {
      const notification = new Notification(title, { body, tag, icon: 'icon-192.png' })
      notification.onclick = () => {
        window.focus()
        notification.close()
      }
    } catch {
      // Some browsers (Chrome on Android) only allow notifications from a service worker.
      this.#save(false)
    }
  }

  #save(enabled: boolean): void {
    this.enabled = enabled
    writeStored(STORAGE_KEY, enabled ? '1' : '0')
  }
}

export const desktop = new DesktopNotifications()
