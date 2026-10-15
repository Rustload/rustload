<script lang="ts">
  let { name, size = 'md' }: { name: string; size?: 'sm' | 'md' } = $props()

  // "AllDebrid" → "AD", "Real-Debrid" → "RD", "TorBox" → "TB".
  const initials = $derived(
    (name.match(/[A-Z0-9]/g) ?? [name[0] ?? '?']).slice(0, 2).join('').toUpperCase() || name.slice(0, 2).toUpperCase(),
  )
</script>

<span
  aria-hidden="true"
  class="grid shrink-0 place-items-center rounded-control border border-line bg-sunken heading font-semibold text-fg-muted
    {size === 'sm' ? 'size-7 text-micro' : 'size-9 text-xs'}"
>
  {initials}
</span>
