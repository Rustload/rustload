import { apiUrl, request } from './client'
import type {
  Account,
  AddOptions,
  AddRequest,
  AuthStatus,
  Category,
  CategoryInput,
  DevicePoll,
  DeviceStart,
  DiagnosticCheck,
  LinkInfo,
  LogEntry,
  LogLevel,
  MediaStatus,
  MoveTarget,
  NotificationTarget,
  Package,
  PackagePatch,
  Provider,
  RestoreResult,
  Settings,
  Stats,
  System,
  User,
} from './types'

/** Body of `POST /api/accounts`: an API key, or a username and password for `credentials` providers. */
export type AccountInput = { provider: string; label?: string; api_key?: string; username?: string; password?: string }

export { ApiError, apiUrl, onUnauthorized, setTransport } from './client'
export type * from './types'

type Packages = { packages: Package[] }

const get = <T>(path: string, query?: Parameters<typeof apiUrl>[1]) => request<T>('GET', apiUrl(path, query))
const post = <T = void>(path: string, body?: unknown) => request<T>('POST', apiUrl(path), body)
const put = <T = void>(path: string, body?: unknown) => request<T>('PUT', apiUrl(path), body)
const patch = <T = void>(path: string, body?: unknown) => request<T>('PATCH', apiUrl(path), body)
const del = (path: string, query?: Parameters<typeof apiUrl>[1]) => request<void>('DELETE', apiUrl(path, query))

export const api = {
  auth: {
    status: () => get<AuthStatus>('auth/status'),
    setup: (username: string, password: string) => post<{ user: User }>('auth/setup', { username, password }),
    login: (username: string, password: string, remember: boolean) =>
      post<{ user: User }>('auth/login', { username, password, remember }),
    logout: () => post('auth/logout'),
    changePassword: (current_password: string, new_password: string) =>
      put('auth/password', { current_password, new_password }),
  },

  system: {
    get: () => get<System>('system'),
    installExtractor: () => post<System>('extraction/7zip/install'),
    pause: () => post<System>('queue/pause'),
    resume: () => post<System>('queue/resume'),
  },

  packages: {
    list: () => get<Packages>('packages'),
    add: (req: AddRequest) => post<Packages>('packages', req),
    addBatch: (packages: AddRequest[]) => post<Packages>('packages/batch', { packages }),
    addTorrents: (files: File[], options: AddOptions) => {
      const form = new FormData()
      for (const file of files) form.append('file', file, file.name)
      form.append('options', JSON.stringify(options))
      return post<Packages>('packages/torrent', form)
    },
    update: (id: number, changes: PackagePatch) => patch<Package>(`packages/${id}`, changes),
    remove: (id: number, files: boolean) => del(`packages/${id}`, { files: files || undefined }),
    pause: (id: number) => post<Package>(`packages/${id}/pause`),
    resume: (id: number) => post<Package>(`packages/${id}/resume`),
    retry: (id: number) => post<Package>(`packages/${id}/retry`),
    extract: (id: number) => post<Package>(`packages/${id}/extract`),
    move: (id: number, to: MoveTarget) => post(`packages/${id}/move`, { to }),
    reorder: (ids: number[]) => post('packages/reorder', { ids }),
  },

  downloads: {
    pause: (id: number) => post(`downloads/${id}/pause`),
    resume: (id: number) => post(`downloads/${id}/resume`),
    retry: (id: number) => post(`downloads/${id}/retry`),
    remove: (id: number) => del(`downloads/${id}`),
  },

  history: {
    list: (q: string, limit: number, offset: number) =>
      get<{ items: Package[]; total: number }>('history', { q, limit, offset }),
    clear: () => del('history'),
    remove: (id: number, files: boolean) => del(`history/${id}`, { files: files || undefined }),
    redownload: (id: number) => post<Packages>(`history/${id}/redownload`),
  },

  categories: {
    list: () => get<{ categories: Category[]; download_root: string }>('categories'),
    create: (input: CategoryInput) => post<Category>('categories', input),
    update: (id: number, input: CategoryInput) => put<Category>(`categories/${id}`, input),
    remove: (id: number) => del(`categories/${id}`),
    reorder: (ids: number[]) => post('categories/reorder', { ids }),
  },

  accounts: {
    list: () => get<{ accounts: Account[]; providers: Provider[] }>('accounts'),
    create: (input: AccountInput) => post<Account>('accounts', { ...input, label: input.label || undefined }),
    update: (
      id: number,
      changes: { label?: string; enabled?: boolean; api_key?: string; username?: string; password?: string },
    ) => put<Account>(`accounts/${id}`, changes),
    remove: (id: number) => del(`accounts/${id}`),
    check: (id: number) => post<Account>(`accounts/${id}/check`),
    reorder: (ids: number[]) => post('accounts/reorder', { ids }),
    deviceStart: (provider: string) => post<DeviceStart>(`accounts/device/${provider}/start`),
    devicePoll: (provider: string, poll_token: string) =>
      post<DevicePoll>(`accounts/device/${provider}/poll`, { poll_token }),
  },

  logs: {
    list: (limit: number, level?: LogLevel) => get<{ entries: LogEntry[] }>('logs', { limit, level }),
    clear: () => del('logs'),
    fileUrl: () => apiUrl('logs/file'),
  },

  settings: {
    get: () => get<Settings>('settings'),
    save: (settings: Settings) => put<Settings>('settings', settings),
    regenerateApiKey: () => post<{ api_key: string }>('settings/api-key'),
  },

  notifications: {
    test: (target: NotificationTarget) => post('notifications/test', target),
  },

  links: {
    check: (urls: string[], scan_pages = false) =>
      post<{ results: LinkInfo[] }>('links/check', { urls, scan_pages: scan_pages || undefined }),
  },

  media: {
    status: () => get<MediaStatus>('media/status'),
    install: () => post<MediaStatus>('media/install'),
    installFfmpeg: () => post<MediaStatus>('media/ffmpeg/install'),
  },

  stats: {
    get: (days: number) => get<Stats>('stats', { days }),
  },

  tools: {
    diagnostics: () => get<{ checks: DiagnosticCheck[] }>('system/diagnostics'),
    backupUrl: () => apiUrl('backup'),
    restore: (backup: unknown, queue: boolean) =>
      request<RestoreResult>('POST', apiUrl('backup/restore', { queue }), backup),
    testTelegram: (token: string) => post<{ username: string }>('integrations/telegram/test', { token }),
  },

  eventsUrl: () => apiUrl('events'),
}
