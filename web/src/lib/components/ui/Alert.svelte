<script lang="ts">
  import type { LucideIcon } from '@lucide/svelte'
  import type { Snippet } from 'svelte'
  import { toneText, toneWash, type Tone } from '$lib/utils/status'

  type Props = {
    /** `bad` is an error, announced as soon as it appears; other tones are a plain callout. */
    tone?: Tone
    icon?: LucideIcon
    class?: string
    children: Snippet
  }

  let { tone = 'bad', icon: Icon, class: className = '', children }: Props = $props()
</script>

<p
  role={tone === 'bad' ? 'alert' : undefined}
  class="flex gap-2.5 rounded-control px-3 py-2 break-words {toneWash[tone]} {toneText[tone]} {className}"
>
  {#if Icon}<Icon size={15} class="mt-0.5 shrink-0" aria-hidden="true" />{/if}
  <span class="min-w-0">{@render children()}</span>
</p>
