import { readStored, writeStored } from '$lib/utils/storage'
import { en, type Messages } from './en'
import { fr } from './fr'

export type Locale = 'en' | 'fr'
export const LOCALES: { id: Locale; name: string }[] = [
  { id: 'en', name: 'English' },
  { id: 'fr', name: 'Français' },
]

export type Plural = { one: string; other: string }
type Leaf = string | Plural
type Keys<T, P extends string = ''> = {
  [K in keyof T & string]: T[K] extends Leaf ? `${P}${K}` : Keys<T[K], `${P}${K}.`>
}[keyof T & string]

export type MessageKey = Keys<Messages>
export type Params = Record<string, string | number>

const STORAGE_KEY = 'rustload.lang'

function isLocale(value: unknown): value is Locale {
  return value === 'en' || value === 'fr'
}

function detect(): Locale {
  const stored = readStored(STORAGE_KEY)
  if (isLocale(stored)) return stored
  return navigator.language?.toLowerCase().startsWith('fr') ? 'fr' : 'en'
}

function flatten(node: object, prefix = '', out = new Map<string, Leaf>()): Map<string, Leaf> {
  for (const [key, value] of Object.entries(node)) {
    const path = prefix + key
    if (typeof value === 'string' || ('one' in value && 'other' in value)) out.set(path, value as Leaf)
    else flatten(value as object, `${path}.`, out)
  }
  return out
}

const flat: Record<Locale, Map<string, Leaf>> = { en: flatten(en), fr: flatten(fr) }
const pluralRules: Record<Locale, Intl.PluralRules> = {
  en: new Intl.PluralRules('en'),
  fr: new Intl.PluralRules('fr'),
}

class I18n {
  locale = $state<Locale>(detect())

  constructor() {
    document.documentElement.lang = this.locale
  }

  /** BCP 47 tag used for Intl formatting. */
  get tag(): string {
    return this.locale === 'fr' ? 'fr-FR' : 'en-GB'
  }

  set(locale: string): void {
    if (!isLocale(locale) || locale === this.locale) return
    this.locale = locale
    document.documentElement.lang = locale
    writeStored(STORAGE_KEY, locale)
  }
}

export const i18n = new I18n()

function interpolate(template: string, params: Params, locale: Locale): string {
  return template.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = params[name]
    if (value === undefined) return match
    return typeof value === 'number' ? value.toLocaleString(locale) : value
  })
}

/** True when `key` exists, for keys that come from the server at runtime. */
export function hasMessage(key: string): key is MessageKey {
  return flat.en.has(key)
}

/** Translates `key`. Plural messages pick their form from `params.count`. */
export function t(key: MessageKey, params?: Params): string {
  const locale = i18n.locale
  const leaf = flat[locale].get(key) ?? flat.en.get(key)
  if (leaf === undefined) return key
  const template =
    typeof leaf === 'string'
      ? leaf
      : pluralRules[locale].select(Number(params?.count ?? 0)) === 'one'
        ? leaf.one
        : leaf.other
  return params ? interpolate(template, params, locale) : template
}

/** Translates `key` and splits it around the `{slot}` placeholder, so markup can go in between. */
export function tSplit(key: MessageKey, slot: string, params?: Params): [string, string] {
  const marker = '\u0000'
  const text = t(key, { ...params, [slot]: marker })
  const at = text.indexOf(marker)
  return at === -1 ? [text, ''] : [text.slice(0, at), text.slice(at + 1)]
}
