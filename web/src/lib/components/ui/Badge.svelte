<script lang="ts">
  import type { LucideIcon } from '@lucide/svelte'
  import { toneFill, toneText, toneWash, type Tone } from '$lib/utils/status'

  type Props = {
    label: string
    tone?: Tone
    icon?: LucideIcon
    /** Shows a status dot; `pulse` animates it for live states. */
    dot?: boolean
    pulse?: boolean
    title?: string
    class?: string
  }

  let { label, tone = 'muted', icon: Icon, dot = false, pulse = false, title, class: className = '' }: Props = $props()
</script>

<span
  {title}
  class="inline-flex h-5 max-w-full shrink-0 items-center gap-1.5 rounded-full px-2 text-xs font-medium whitespace-nowrap
    {toneWash[tone]} {toneText[tone]} {className}"
>
  {#if Icon}
    <Icon size={12} strokeWidth={2.25} aria-hidden="true" />
  {:else if dot}
    <span class="relative flex size-1.5" aria-hidden="true">
      {#if pulse}
        <span class="absolute inset-0 animate-ping rounded-full opacity-60 {toneFill[tone]}"></span>
      {/if}
      <span class="relative size-1.5 rounded-full {toneFill[tone]}"></span>
    </span>
  {/if}
  <span class="truncate">{label}</span>
</span>
