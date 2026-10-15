function currentPath(): string {
  const hash = window.location.hash.replace(/^#/, '')
  return hash.startsWith('/') ? hash : `/${hash}`
}

class Router {
  path = $state(currentPath())
  segments = $derived(this.path.split('/').filter(Boolean))

  constructor() {
    window.addEventListener('hashchange', () => {
      this.path = currentPath()
    })
  }

  go(path: string): void {
    if (path !== this.path) window.location.hash = path
  }

  href(path: string): string {
    return `#${path}`
  }

  isActive(prefix: string): boolean {
    return prefix === '/' ? this.path === '/' : this.path === prefix || this.path.startsWith(`${prefix}/`)
  }
}

export const router = new Router()
