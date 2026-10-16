import { api, onUnauthorized, type AuthStatus } from '$lib/api'

export type AuthPhase = 'loading' | 'unreachable' | 'setup' | 'login' | 'ready'

class Auth {
  status = $state<AuthStatus | null>(null)
  unreachable = $state(false)

  /** `unreachable` only applies before the first successful status; later outages keep the UI. */
  phase: AuthPhase = $derived.by(() => {
    if (!this.status) return this.unreachable ? 'unreachable' : 'loading'
    if (!this.status.auth_enabled) return 'ready'
    if (this.status.setup_required) return 'setup'
    return this.status.user ? 'ready' : 'login'
  })

  username = $derived(this.status?.user?.username ?? null)
  enabled = $derived(this.status?.auth_enabled ?? false)
  /** In the desktop app: no signing out, the window always opens logged in. */
  desktop = $derived(this.status?.desktop ?? false)

  constructor() {
    onUnauthorized(() => {
      if (this.status?.auth_enabled) this.status = { ...this.status, user: null }
    })
  }

  async load(): Promise<void> {
    try {
      this.status = await api.auth.status()
      this.unreachable = false
    } catch {
      this.unreachable = true
    }
  }

  async setup(username: string, password: string): Promise<void> {
    const { user } = await api.auth.setup(username, password)
    this.status = { auth_enabled: true, setup_required: false, user, desktop: this.desktop }
  }

  async login(username: string, password: string, remember: boolean): Promise<void> {
    const { user } = await api.auth.login(username, password, remember)
    this.status = { auth_enabled: true, setup_required: false, user, desktop: this.desktop }
  }

  async logout(): Promise<void> {
    try {
      await api.auth.logout()
    } finally {
      if (this.status) this.status = { ...this.status, user: null }
    }
  }
}

export const auth = new Auth()
