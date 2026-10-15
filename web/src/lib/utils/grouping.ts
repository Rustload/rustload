// Suggests packages for the link grabber: archive sets, folders, playlists and magnets get their
// own package, files with similar names are kept together. Pure functions, no state.

import type { LinkInfo } from '$lib/api'
import { displayName } from './format'

export type GroupMode = 'smart' | 'single'

export type GrabbedLink = {
  url: string
  kind: 'http' | 'magnet'
  /** Null while the link is being checked. */
  info: LinkInfo | null
}

export type GroupKind = 'files' | 'archive' | 'folder' | 'playlist' | 'magnet'

export type LinkGroup = {
  /** Stable across re-renders so renamed groups keep their name while links are checked. */
  key: string
  kind: GroupKind
  /** Empty when nothing better than the server's own default is known. */
  name: string
  links: GrabbedLink[]
}

const SEPARATOR = /[._ \-[(]/
const TRAILING_SEPARATORS = /[._ \-[(]+$/
/** `name.part01.rar`, `name.7z.001` / `name.zip.001` / `name.001`, `name.r00` and its `name.rar`. */
const VOLUME_PATTERNS = [/^(.*)\.part\d+\.rar$/i, /^(.*?)(?:\.(?:7z|zip|rar|tar))?\.\d{3}$/i, /^(.*)\.(?:r\d{2}|rar)$/i]

/** File name shown for a link: the checked name, else the last URL segment. */
export function linkName(link: GrabbedLink): string {
  const info = link.info
  return info?.filename ?? info?.folder?.name ?? info?.media?.title ?? displayName({ filename: null, url: link.url })
}

/** Name of a multi-volume archive set without its volume suffix, or null for anything else. */
function archiveStem(filename: string): string | null {
  for (const pattern of VOLUME_PATTERNS) {
    const match = filename.match(pattern)
    if (match?.[1]) return match[1]
  }
  return null
}

/** Key shared by every volume of a multi-part archive, or null for a standalone file. */
export function archiveSetKey(filename: string): string | null {
  return archiveStem(filename)?.toLowerCase() ?? null
}

/** `video.mkv` → `video`, `data.tar.gz` → `data`; `Blade.Runner.2049` and `Episode 100` stay whole. */
function stripExtension(filename: string): string {
  const lower = filename.toLowerCase()
  const double = ['.tar.gz', '.tar.bz2', '.tar.xz'].find((suffix) => lower.endsWith(suffix))
  if (double) return filename.slice(0, -double.length)
  const match = filename.match(/^(.+)\.([A-Za-z0-9]{1,5})$/)
  return match && /[A-Za-z]/.test(match[2]) ? match[1] : filename
}

/** `Movie.2024.part01.rar` → `Movie.2024`, `backup.7z.001` → `backup`, `video.mkv` → `video`. */
export function packageNameFromFile(filename: string): string {
  const stem = archiveStem(filename) ?? stripExtension(filename)
  return stem.replace(TRAILING_SEPARATORS, '') || filename
}

/** Name of a link as a package name: titles of media links are kept as they are. */
function stemOf(link: GrabbedLink): string {
  const name = linkName(link)
  return link.info?.handler.type === 'media' ? name : packageNameFromFile(name)
}

/** Longest common prefix of several names, cut back to a word boundary; null when not meaningful. */
export function commonName(names: string[]): string | null {
  if (names.length < 2) return null
  const first = names[0]
  let length = Math.min(...names.map((n) => n.length))
  for (const name of names.slice(1)) {
    let i = 0
    while (i < length && name[i].toLowerCase() === first[i].toLowerCase()) i++
    length = i
  }
  // A prefix that ends where every name ends or breaks at a separator is whole; otherwise it stops
  // mid-word and goes back to the previous separator.
  const whole = names.every((n) => n.length === length || SEPARATOR.test(n[length]))
  let prefix = first.slice(0, length)
  if (!whole) prefix = prefix.replace(/[^._ \-[(]*$/, '')
  prefix = prefix.replace(TRAILING_SEPARATORS, '')
  return prefix.length >= 4 ? prefix : null
}

function looseName(links: GrabbedLink[]): string {
  const stems = links.map(stemOf)
  return commonName(stems) ?? stems[0] ?? ''
}

/** Episodes, parts and numbered files: the same name once digits are ignored. */
function digitsMasked(stem: string): string {
  return stem.toLowerCase().replace(/\d+/g, '#')
}

/** Puts files whose names belong together (`Show.S01E01…`, `Show.S01E02…`) in one package. */
function clusterBySimilarName(links: GrabbedLink[]): GrabbedLink[][] {
  const clusters: { stems: string[]; links: GrabbedLink[] }[] = []
  for (const link of links) {
    const stem = stemOf(link)
    const home = clusters.find((cluster) => {
      if (cluster.stems.some((other) => digitsMasked(other) === digitsMasked(stem))) return true
      const shared = commonName([...cluster.stems, stem])
      const shortest = Math.min(stem.length, ...cluster.stems.map((s) => s.length))
      return shared !== null && shared.length >= Math.min(8, shortest / 2)
    })
    if (home) {
      home.stems.push(stem)
      home.links.push(link)
    } else {
      clusters.push({ stems: [stem], links: [link] })
    }
  }
  return clusters.map((cluster) => cluster.links)
}

/** Groups links into the packages that will be created. */
export function groupLinks(links: GrabbedLink[], mode: GroupMode): LinkGroup[] {
  const groups: LinkGroup[] = []
  const loose: GrabbedLink[] = []
  const archives = new Map<string, GrabbedLink[]>()

  for (const link of links) {
    const info = link.info
    if (link.kind === 'magnet') {
      const name = info?.magnet?.name ?? info?.filename ?? ''
      groups.push({ key: `magnet:${link.url}`, kind: 'magnet', name, links: [link] })
    } else if (info?.folder) {
      const name = info.folder.name ?? displayName({ filename: null, url: link.url })
      groups.push({ key: `folder:${link.url}`, kind: 'folder', name, links: [link] })
    } else if (info?.media?.playlist) {
      const name = info.media.playlist.title ?? info.media.title ?? ''
      groups.push({ key: `playlist:${link.url}`, kind: 'playlist', name, links: [link] })
    } else if (mode === 'smart') {
      const key = info?.filename && info.handler.type !== 'media' ? archiveSetKey(info.filename) : null
      if (key) archives.set(key, [...(archives.get(key) ?? []), link])
      else loose.push(link)
    } else {
      loose.push(link)
    }
  }

  for (const [key, members] of archives) {
    if (members.length < 2) {
      loose.push(...members)
      continue
    }
    groups.push({ key: `archive:${key}`, kind: 'archive', name: stemOf(members[0]), links: members })
  }

  const order = new Map(links.map((link, i) => [link, i]))
  loose.sort((a, b) => order.get(a)! - order.get(b)!)
  if (mode === 'single' && loose.length) {
    groups.push({ key: 'files', kind: 'files', name: looseName(loose), links: loose })
  } else {
    // Keyed by the first link so a renamed package keeps its name while the others are checked.
    for (const members of clusterBySimilarName(loose)) {
      groups.push({ key: `files:${members[0].url}`, kind: 'files', name: looseName(members), links: members })
    }
  }
  // Packages appear in the order their first link was pasted.
  return groups.sort((a, b) => order.get(a.links[0])! - order.get(b.links[0])!)
}
