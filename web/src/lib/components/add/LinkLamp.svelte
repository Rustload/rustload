<script lang="ts">
  import { linkTone, toneFill } from '$lib/utils/status'

  /** Status dot of the link grabber: hollow while nothing is known, pulsing while checking. */
  type Props = {
    state: keyof typeof linkTone
    label: string
  }

  let { state, label }: Props = $props()

  const fill = $derived(
    linkTone[state] === 'muted' ? 'border-fg-faint' : `border-transparent ${toneFill[linkTone[state]]}`,
  )
</script>

<span class="relative size-2 shrink-0 rounded-full border {fill}" role="img" aria-label={label} title={label}>
  {#if state === 'checking'}
    <span class="absolute -inset-1 animate-ping rounded-full border border-fg-faint opacity-50"></span>
  {/if}
</span>
