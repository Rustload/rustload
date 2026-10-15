/** DOM helpers shared by the extension pages. */

import { t } from './i18n.js'
import { icon } from './icons.js'

export const $ = (id) => document.getElementById(id)

export function el(tag, className, text) {
  const node = document.createElement(tag)
  if (className) node.className = className
  if (text != null) node.textContent = text
  return node
}

export function iconSlot(name, { size = 15, className = '' } = {}) {
  const slot = el('span', `icon-slot ${className}`.trim())
  slot.append(icon(name, { size }))
  return slot
}

/** Fills a `.feedback` box with `text` and the icon of its tone (`ok`, `warn` or `bad`), and shows it. */
export function showFeedback(box, tone, text) {
  box.className = `feedback tone-${tone}`
  box.replaceChildren(iconSlot(tone === 'ok' ? 'circle-check' : 'circle-alert', { size: 14 }), el('span', '', text))
  box.hidden = false
}

/**
 * Runs `task` with `button` disabled and showing a spinner, then puts its content back. The caller
 * decides when the button is enabled again.
 */
export async function withSpinner(button, task) {
  const content = [...button.childNodes]
  button.disabled = true
  button.replaceChildren(el('span', 'spinner'))
  try {
    return await task()
  } finally {
    button.replaceChildren(...content)
  }
}

/**
 * Options for a category select: Automatic, the server's categories (`null` when unknown), then
 * the saved category if the server doesn't list it. `missingLabel` names that last one when the
 * list is known.
 */
export function categoryOptions(categories, { categoryId, categoryName }, missingLabel = (name) => name) {
  const options = [new Option(t('categoryAuto'), '')]
  for (const category of categories ?? []) options.push(new Option(category.name, String(category.id)))
  if (categoryId != null && !options.some((option) => option.value === String(categoryId))) {
    const name = categoryName || `#${categoryId}`
    options.push(new Option(categories ? missingLabel(name) : name, String(categoryId)))
  }
  return options
}
