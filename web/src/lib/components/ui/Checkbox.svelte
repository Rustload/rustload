<script lang="ts">
  type Props = {
    checked: boolean
    label: string
    disabled?: boolean
    onchange?: (checked: boolean) => void
    class?: string
  }

  let { checked = $bindable(), label, disabled = false, onchange, class: className = '' }: Props = $props()
</script>

<label class="inline-flex items-center gap-2.5 text-sm {disabled ? 'opacity-60' : 'cursor-pointer'} {className}">
  <input
    type="checkbox"
    bind:checked
    {disabled}
    onchange={() => onchange?.(checked)}
    class="size-4 shrink-0 cursor-[inherit] rounded-chip accent-[var(--fg)]"
  />
  <span>{label}</span>
</label>
