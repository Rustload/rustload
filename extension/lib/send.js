import { ApiError, createClient } from './api.js'
import { t } from './i18n.js'
import { hasHostPermission, loadSettings, saveSettings } from './settings.js'

/**
 * Sends links/magnets to Rustload (`POST /api/packages`) with the saved defaults, unless
 * `paused` / `categoryId` are given. Returns `{ packages, settings, notice }`, `notice` being
 * something to tell the user although the links were added; throws `ApiError`.
 */
export async function sendLinks(links, { paused, categoryId } = {}) {
  const settings = await loadSettings()
  if (!settings.serverUrl) throw new ApiError('unconfigured')
  if (!(await hasHostPermission(settings.serverUrl))) throw new ApiError('permission')
  if (!links.length) throw new ApiError('rejected', { code: 'NO_LINKS' })
  const client = createClient(settings)
  const options = {
    paused: paused ?? settings.addPaused,
    categoryId: categoryId === undefined ? settings.categoryId : categoryId,
  }
  try {
    return { packages: await client.add(links, options), settings, notice: '' }
  } catch (error) {
    if (error?.code !== 'UNKNOWN_CATEGORY' || options.categoryId == null) throw error
  }
  // The category was deleted on the server: add the links with the automatic one, and forget it
  // as the default so that the next sends don't fail first.
  const packages = await client.add(links, { ...options, categoryId: null })
  const wasDefault = options.categoryId === settings.categoryId
  if (wasDefault) await saveSettings({ categoryId: null, categoryName: '' })
  const name = (wasDefault && settings.categoryName) || `#${options.categoryId}`
  return { packages, settings, notice: t('categoryGone', { name }) }
}
