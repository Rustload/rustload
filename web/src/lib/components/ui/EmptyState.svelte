<script lang="ts">
  import type { LucideIcon } from '@lucide/svelte'
  import type { Snippet } from 'svelte'

  type Props = {
    icon: LucideIcon
    title: string
    body?: string
    action?: Snippet
    class?: string
  }

  let { icon: Icon, title, body, action, class: className = '' }: Props = $props()
</script>

<div class="flex flex-col items-center px-6 py-14 text-center {className}">
  <div class="mb-4 grid size-12 place-items-center rounded-full border border-line text-fg-faint">
    <Icon size={22} strokeWidth={1.75} aria-hidden="true" />
  </div>
  <h2 class="heading text-base font-semibold">{title}</h2>
  {#if body}
    <p class="mt-1.5 max-w-[42ch] text-fg-muted">{body}</p>
  {/if}
  {#if action}
    <div class="mt-5">{@render action()}</div>
  {/if}
</div>
