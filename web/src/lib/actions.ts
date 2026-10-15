// User actions that need a confirmation step, shared by the queue, dashboard and history views.

import { api, type Download, type Package } from '$lib/api'
import { t } from '$lib/i18n/index.svelte'
import { history } from '$lib/stores/history.svelte'
import { queue } from '$lib/stores/queue.svelte'
import { ui } from '$lib/stores/ui.svelte'
import { displayName } from '$lib/utils/format'

export async function deletePackages(packages: Package[]): Promise<boolean> {
  if (packages.length === 0) return false
  const { confirmed, checked } = await ui.confirm({
    title: t('queue.deleteTitle', { count: packages.length, name: packages[0].name }),
    body: t('queue.deleteBody'),
    checkbox: t('queue.deleteFiles'),
    confirmLabel: t('common.delete'),
    danger: true,
  })
  if (!confirmed) return false
  return queue.run(() => Promise.all(packages.map((p) => api.packages.remove(p.id, checked))))
}

export async function deleteFile(file: Download): Promise<boolean> {
  const { confirmed } = await ui.confirm({
    title: t('queue.deleteFileTitle', { name: displayName(file) }),
    body: t('queue.deleteFileBody'),
    confirmLabel: t('common.delete'),
    danger: true,
  })
  if (!confirmed) return false
  return queue.run(() => api.downloads.remove(file.id))
}

export async function removeHistoryEntry(pkg: Package): Promise<void> {
  const { confirmed, checked } = await ui.confirm({
    title: t('history.removeTitle', { name: pkg.name }),
    checkbox: t('history.removeFiles'),
    confirmLabel: t('history.removeEntry'),
    danger: true,
  })
  if (confirmed) await history.run(() => api.history.remove(pkg.id, checked))
}

export async function clearHistory(): Promise<void> {
  const { confirmed } = await ui.confirm({
    title: t('history.clearTitle'),
    body: t('history.clearBody'),
    confirmLabel: t('history.clear'),
    danger: true,
  })
  if (confirmed) await history.run(() => api.history.clear())
}
