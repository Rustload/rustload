<script lang="ts">
  import { MARK, MARK_SMALL, WORDMARK } from '$lib/brand'

  /**
   * The Rustload logo (docs/brand). `size` is the height of the mark in px. `withName` adds the
   * wordmark at the lockup proportions; `tile` draws the app icon instead (white mark on oxide).
   * Up to 20 px the mark switches to its pixel-aligned small variant.
   */
  type Props = { size?: number; withName?: boolean; tile?: boolean; class?: string }
  let { size = 28, withName = false, tile = false, class: className = '' }: Props = $props()

  // Lockup units: the mark is 1.3 times the wordmark's ascender height, centred on it (its tip lands
  // below the baseline), then a gap of 0.3 of the ascender height.
  const A = WORDMARK.ascender
  const M = 1.3 * A
  const TOP = -A - (M - A) / 2
  const TEXT_X = M + 0.3 * A - WORDMARK.x
  const WIDTH = TEXT_X + WORDMARK.x + WORDMARK.width

  const small = $derived(size <= 20)
  const markTransform = $derived(small ? `scale(${M / 16})` : `scale(${M / 28}) translate(-2 -2)`)
</script>

{#if tile}
  <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true" class="shrink-0 {className}">
    <rect width="32" height="32" rx="7" fill="var(--color-oxide)" />
    <path fill="#fff" transform="translate(6.13 6.13) scale(0.617)" d={MARK} />
  </svg>
{:else if withName}
  <svg
    width={(size * WIDTH) / M}
    height={size}
    viewBox="0 {TOP} {WIDTH} {M}"
    role="img"
    aria-label="Rustload"
    class="shrink-0 {className}"
  >
    <path fill="var(--brand)" transform="translate(0 {TOP}) {markTransform}" d={small ? MARK_SMALL : MARK} />
    <path fill="currentColor" transform="translate({TEXT_X} 0)" d={WORDMARK.d} />
  </svg>
{:else}
  <svg
    width={size}
    height={size}
    viewBox={small ? '0 0 16 16' : '2 2 28 28'}
    aria-hidden="true"
    class="shrink-0 {className}"
  >
    <path fill="var(--brand)" d={small ? MARK_SMALL : MARK} />
  </svg>
{/if}
