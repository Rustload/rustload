import { router } from '$lib/router.svelte'

export type ConfirmOptions = {
  title: string
  body?: string
  confirmLabel: string
  danger?: boolean
  /** Optional checkbox shown under the message (e.g. "also delete files"). */
  checkbox?: string
}

export type ConfirmResult = { confirmed: boolean; checked: boolean }

type PendingConfirm = ConfirmOptions & { resolve: (result: ConfirmResult) => void }

class Ui {
  addOpen = $state(false)
  addText = $state('')
  addFiles = $state<File[]>([])
  /** False for links handed over by another page (bookmarklet, share target): the grabber asks
   *  before contacting them, so a crafted link can't make the server request internal addresses. */
  addTrusted = $state(true)
  paletteOpen = $state(false)
  shortcutsOpen = $state(false)
  /** Package the queue page should reveal (scroll to, expand and highlight) once. */
  focusPackage = $state<number | null>(null)
  confirmRequest = $state<PendingConfirm | null>(null)

  openAdd(text = '', files: File[] = [], trusted = true): void {
    this.addText = text
    this.addFiles = files
    this.addTrusted = trusted
    this.addOpen = true
  }

  /** Opens the queue on a package: scrolled to, unfolded and outlined for a moment. */
  revealPackage(id: number): void {
    this.focusPackage = id
    router.go('/queue')
  }

  /** Opens the shared confirmation dialog and resolves when the user answers. */
  confirm(options: ConfirmOptions): Promise<ConfirmResult> {
    this.confirmRequest?.resolve({ confirmed: false, checked: false })
    return new Promise((resolve) => {
      this.confirmRequest = { ...options, resolve }
    })
  }

  answerConfirm(result: ConfirmResult): void {
    const request = this.confirmRequest
    this.confirmRequest = null
    request?.resolve(result)
  }
}

export const ui = new Ui()

class Clock {
  now = $state(Date.now())

  constructor() {
    setInterval(() => (this.now = Date.now()), 1000)
  }
}

/** Shared 1 Hz clock for countdowns and finish times. */
export const clock = new Clock()
