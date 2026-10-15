/*
 * Functions injected into web pages with `scripting.executeScript` (activeTab). They are
 * serialised, so each one must be self-contained: no imports, no outer variables.
 */

/** Links (and their text) of the page: anchors, image maps and media sources. */
export function collectPageLinks() {
  const seen = new Map()
  const here = location.href.replace(/#.*$/, '')
  const add = (raw, text) => {
    if (typeof raw !== 'string' || !raw) return
    let url
    try {
      url = new URL(raw, document.baseURI)
    } catch {
      return
    }
    let key
    if (url.protocol === 'magnet:') key = raw.trim()
    else if (url.protocol === 'http:' || url.protocol === 'https:') key = url.href.replace(/#.*$/, '')
    else return
    if (key === here) return
    const label = String(text ?? '')
      .replace(/\s+/g, ' ')
      .trim()
      .slice(0, 160)
    const known = seen.get(key)
    if (!known) seen.set(key, { url: key, text: label })
    else if (!known.text && label) known.text = label
  }
  for (const a of document.querySelectorAll('a[href], area[href]')) {
    if (typeof a.href !== 'string') continue // SVG links
    add(a.href, a.textContent || a.title || a.querySelector('img[alt]')?.alt || '')
  }
  for (const media of document.querySelectorAll('video[src], audio[src], source[src]')) add(media.src, '')
  return { url: location.href, title: document.title, links: [...seen.values()].slice(0, 5000) }
}

/** Links inside the current selection: hrefs of selected anchors plus URLs written in the text. */
export function collectSelectionLinks() {
  const selection = getSelection()
  const found = []
  if (!selection) return found
  for (let i = 0; i < selection.rangeCount; i++) {
    const range = selection.getRangeAt(i)
    const node = range.commonAncestorContainer
    const scope = node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement
    if (!scope) continue
    const around = scope.closest('a[href]')
    if (around && typeof around.href === 'string') found.push(around.href)
    for (const a of scope.querySelectorAll('a[href]')) {
      if (typeof a.href === 'string' && range.intersectsNode(a)) found.push(a.href)
    }
  }
  found.push(selection.toString())
  return found
}
