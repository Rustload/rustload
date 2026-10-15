// `navigator.platform` is deprecated: Chromium browsers expose `userAgentData`, the others the user agent.
const nav = navigator as Navigator & { userAgentData?: { platform: string } }

/** Apple devices show ⌘ where the others show Ctrl. */
export const isApple = /mac|iphone|ipad|ipod/i.test(nav.userAgentData?.platform || nav.userAgent)
