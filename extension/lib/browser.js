/**
 * The WebExtension API namespace. Firefox exposes the promise-based `browser`; Chromium browsers
 * expose `chrome`, whose Manifest V3 APIs return promises as well when no callback is given.
 */
export const ext = globalThis.browser ?? globalThis.chrome
