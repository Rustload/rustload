import { mount } from 'svelte'
import './app.css'
import App from './App.svelte'
// Registers the install-prompt listener before the browser fires it.
import './lib/stores/pwa.svelte'

async function start() {
  // Dev-only fake backend (`pnpm dev:mock`); stripped from production builds.
  if (import.meta.env.DEV && import.meta.env.MODE === 'mock') {
    const { installMock } = await import('./mock')
    installMock()
  }
  mount(App, { target: document.getElementById('app')! })
}

start()
