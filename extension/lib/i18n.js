import { ext } from './browser.js'

/** Language of the messages actually used ("en" or "fr"), for `lang` and number formatting. */
export const locale = ext.i18n.getMessage('localeCode') || 'en'

const plurals = new Intl.PluralRules(locale)
const integers = new Intl.NumberFormat(locale)

/**
 * Translated message. Named parameters map to the `$NAME$` placeholders of messages.json, which
 * are numbered in alphabetical order of their names.
 */
export function t(key, params = {}) {
  const names = Object.keys(params).sort()
  return ext.i18n.getMessage(key, names.map((name) => String(params[name]))) || key
}

/** Picks `key_one` or `key_other` with the language's plural rules (French: 0 and 1 are singular). */
export function tn(key, count, params = {}) {
  const form = plurals.select(count) === 'one' ? 'one' : 'other'
  return t(`${key}_${form}`, { ...params, count: integers.format(count) })
}

/** Fills `data-i18n` (text) and `data-i18n-placeholder|title|aria-label` attributes. */
export function localize(root = document) {
  for (const el of root.querySelectorAll('[data-i18n]')) el.textContent = t(el.dataset.i18n)
  for (const attr of ['placeholder', 'title', 'aria-label']) {
    for (const el of root.querySelectorAll(`[data-i18n-${attr}]`)) {
      el.setAttribute(attr, t(el.getAttribute(`data-i18n-${attr}`)))
    }
  }
  if (root === document) {
    document.documentElement.lang = locale
    const title = document.querySelector('title[data-i18n]')
    if (title) document.title = title.textContent
  }
}
