export type ParsedLinks = { links: string[]; magnets: string[] }

const HTTP_RE = /^https?:\/\/\S+$/i
const MAGNET_RE = /^magnet:\?\S+$/i

/** Splits pasted text into http(s) links and magnets, ignoring anything else and duplicates. */
export function parseLinks(text: string): ParsedLinks {
  const links = new Set<string>()
  const magnets = new Set<string>()
  for (const token of text.split(/\s+/)) {
    const value = token.trim().replace(/^[<("']+|[>)"',;]+$/g, '')
    if (MAGNET_RE.test(value)) magnets.add(value)
    else if (HTTP_RE.test(value) && URL.canParse(value)) links.add(value)
  }
  return { links: [...links], magnets: [...magnets] }
}

export function containsLinks(text: string): boolean {
  const { links, magnets } = parseLinks(text)
  return links.length + magnets.length > 0
}

export function isTorrentFile(file: File): boolean {
  return file.name.toLowerCase().endsWith('.torrent')
}
