<script lang="ts">
  import { toneFill, type Tone } from '$lib/utils/status'

  type Props = {
    /** 0..1, or null for an indeterminate bar. */
    value: number | null
    tone?: Tone
    size?: 'xs' | 'sm' | 'md' | 'lg'
    label?: string
    class?: string
  }

  let { value, tone = 'signal', size = 'sm', label, class: className = '' }: Props = $props()

  const heights = { xs: 'h-0.5', sm: 'h-1', md: 'h-1.5', lg: 'h-2' }
  const pct = $derived(value === null ? null : Math.round(Math.min(1, Math.max(0, value)) * 1000) / 10)
</script>

<div
  role="progressbar"
  aria-label={label}
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={pct ?? undefined}
  class="relative w-full overflow-hidden rounded-full bg-sunken {heights[size]} {className}"
>
  {#if pct === null}
    <div class="indeterminate absolute inset-y-0 w-1/3 rounded-full {toneFill[tone]}"></div>
  {:else}
    <div
      class="h-full rounded-full transition-[width] duration-700 ease-linear {toneFill[tone]}"
      style:width="{pct}%"
    ></div>
  {/if}
</div>

<style>
  .indeterminate {
    animation: slide 1.4s ease-in-out infinite;
  }
  @keyframes slide {
    from {
      left: -33%;
    }
    to {
      left: 100%;
    }
  }
</style>
