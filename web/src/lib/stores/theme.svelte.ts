import { readStored, writeStored } from '$lib/utils/storage'

export type ThemePreference = 'dark' | 'light' | 'system'

const STORAGE_KEY = 'rustload.theme'
const media = window.matchMedia('(prefers-color-scheme: dark)')

function stored(): ThemePreference {
  const value = readStored(STORAGE_KEY)
  return value === 'dark' || value === 'light' ? value : 'system'
}

class Theme {
  preference = $state<ThemePreference>(stored())
  systemDark = $state(media.matches)
  dark = $derived(this.preference === 'dark' || (this.preference === 'system' && this.systemDark))

  constructor() {
    media.addEventListener('change', (event) => {
      this.systemDark = event.matches
      this.#apply()
    })
    this.#apply()
  }

  set(preference: ThemePreference): void {
    this.preference = preference
    writeStored(STORAGE_KEY, preference)
    this.#apply()
  }

  #apply(): void {
    document.documentElement.classList.toggle('dark', this.dark)
  }
}

export const theme = new Theme()
