import type { DiagnosticCheck, DownloadStatus, LinkInfo, LogLevel, PackageStatus } from '$lib/api'
import type { ToastLevel } from '$lib/stores/toasts.svelte'

export type Tone = 'signal' | 'ok' | 'bad' | 'info' | 'warn' | 'muted'

export const packageTone: Record<PackageStatus, Tone> = {
  queued: 'muted',
  downloading: 'signal',
  paused: 'muted',
  debrid: 'info',
  verifying: 'info',
  extracting: 'info',
  completed: 'ok',
  failed: 'bad',
}

export const downloadTone: Record<DownloadStatus, Tone> = {
  queued: 'muted',
  resolving: 'info',
  downloading: 'signal',
  paused: 'muted',
  completed: 'ok',
  failed: 'bad',
  skipped: 'warn',
}

export const toastTone: Record<ToastLevel, Tone> = {
  success: 'ok',
  info: 'info',
  warning: 'warn',
  error: 'bad',
}

export const logTone: Record<LogLevel, Tone> = {
  ERROR: 'bad',
  WARN: 'warn',
  INFO: 'info',
  DEBUG: 'muted',
}

/** System diagnostics. */
export const checkTone: Record<DiagnosticCheck['status'], Tone> = {
  ok: 'ok',
  warn: 'warn',
  error: 'bad',
}

/** The link grabber's lamp, `checking` while the server looks the link up. Muted lamps are drawn hollow. */
export const linkTone: Record<LinkInfo['status'] | 'checking', Tone> = {
  checking: 'muted',
  online: 'ok',
  offline: 'bad',
  unknown: 'muted',
  unsupported: 'warn',
}

/** Text colour class per tone. */
export const toneText: Record<Tone, string> = {
  signal: 'text-signal-ink',
  ok: 'text-ok',
  bad: 'text-bad',
  info: 'text-info',
  warn: 'text-warn',
  muted: 'text-fg-muted',
}

/** Solid fill class per tone (progress bars, dots). */
export const toneFill: Record<Tone, string> = {
  signal: 'bg-signal',
  ok: 'bg-ok',
  bad: 'bg-bad',
  info: 'bg-info',
  warn: 'bg-warn',
  muted: 'bg-fg-faint',
}

/** Subtle background class per tone (badges, banners). */
export const toneWash: Record<Tone, string> = {
  signal: 'bg-signal-wash',
  ok: 'bg-ok-wash',
  bad: 'bg-bad-wash',
  info: 'bg-info-wash',
  warn: 'bg-warn-wash',
  muted: 'bg-sunken',
}

export function isActiveDownload(status: DownloadStatus): boolean {
  return status === 'downloading' || status === 'resolving'
}

export function isFinishedDownload(status: DownloadStatus): boolean {
  return status === 'completed' || status === 'skipped'
}
