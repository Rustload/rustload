// Per-browser preferences in localStorage, which can be unavailable (blocked storage, some private
// modes): reads then return null and writes only last for the visit.

export function readStored(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

export function writeStored(key: string, value: string): void {
  try {
    localStorage.setItem(key, value)
  } catch {
    /* storage unavailable */
  }
}
