<script lang="ts">
  type Props = {
    checked: boolean
    /** Visible label; when omitted, `ariaLabel` must be given. */
    label?: string
    ariaLabel?: string
    hint?: string
    disabled?: boolean
    onchange?: (checked: boolean) => void
    class?: string
  }

  let {
    checked = $bindable(),
    label,
    ariaLabel,
    hint,
    disabled = false,
    onchange,
    class: className = '',
  }: Props = $props()

  const id = $props.id()

  function toggle() {
    checked = !checked
    onchange?.(checked)
  }
</script>

{#snippet track()}
  <button
    type="button"
    role="switch"
    id="{id}-switch"
    aria-checked={checked}
    aria-label={label ? undefined : ariaLabel}
    aria-labelledby={label ? `${id}-label` : undefined}
    aria-describedby={hint ? `${id}-hint` : undefined}
    {disabled}
    onclick={toggle}
    class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors duration-150
      disabled:opacity-50 {checked ? 'border-fg bg-fg' : 'border-line-strong bg-sunken'}"
  >
    <span
      class="absolute left-0.5 size-3.5 rounded-full transition-transform duration-150
        {checked ? 'translate-x-4 bg-bg' : 'translate-x-0 bg-fg-muted'}"
    ></span>
  </button>
{/snippet}

{#if label}
  <div class="flex items-start justify-between gap-6 {className}">
    <div class="min-w-0">
      <label id="{id}-label" for="{id}-switch" class="block font-medium text-fg">{label}</label>
      {#if hint}
        <p id="{id}-hint" class="mt-0.5 text-small text-fg-muted">{hint}</p>
      {/if}
    </div>
    <div class="pt-0.5">{@render track()}</div>
  </div>
{:else}
  {@render track()}
{/if}
