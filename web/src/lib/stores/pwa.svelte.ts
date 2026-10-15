/** The browser's "install this app" prompt, which only fires once and must be kept for later. */
type InstallPrompt = Event & { prompt: () => Promise<void>; userChoice: Promise<{ outcome: string }> }

class Pwa {
  canInstall = $state(false)
  #prompt: InstallPrompt | null = null

  constructor() {
    window.addEventListener('beforeinstallprompt', (event) => {
      event.preventDefault()
      this.#prompt = event as InstallPrompt
      this.canInstall = true
    })
    window.addEventListener('appinstalled', () => {
      this.#prompt = null
      this.canInstall = false
    })
  }

  async install(): Promise<void> {
    const prompt = this.#prompt
    if (!prompt) return
    await prompt.prompt()
    await prompt.userChoice
    this.#prompt = null
    this.canInstall = false
  }
}

export const pwa = new Pwa()
