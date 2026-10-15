import { ApiError } from '$lib/api'
import { t } from '$lib/i18n/index.svelte'

export type ToastLevel = 'success' | 'info' | 'warning' | 'error'
export type Toast = { id: number; level: ToastLevel; message: string }

const DURATION: Record<ToastLevel, number> = { success: 3500, info: 5000, warning: 7000, error: 9000 }

class Toasts {
  items = $state<Toast[]>([])
  #next = 1

  push(level: ToastLevel, message: string): void {
    const id = this.#next++
    this.items = [...this.items.slice(-4), { id, level, message }]
    setTimeout(() => this.dismiss(id), DURATION[level])
  }

  success(message: string): void {
    this.push('success', message)
  }

  /** Shows an error toast for anything thrown by an API call. */
  error(err: unknown): void {
    this.push('error', errorMessage(err))
  }

  dismiss(id: number): void {
    this.items = this.items.filter((toast) => toast.id !== id)
  }
}

export function errorMessage(err: unknown): string {
  if (err instanceof ApiError) {
    if (err.code === 'NETWORK') return t('errors.network')
    return err.message || t('errors.generic', { message: err.code })
  }
  return t('errors.generic', { message: err instanceof Error ? err.message : String(err) })
}

export const toasts = new Toasts()

/** Runs a user action and shows an error toast if it fails; resolves to whether it succeeded. */
export async function runWithToast(action: () => Promise<unknown>): Promise<boolean> {
  try {
    await action()
    return true
  } catch (err) {
    toasts.error(err)
    return false
  }
}
