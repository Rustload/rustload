<script lang="ts">
  import { t } from '$lib/i18n/index.svelte'

  /** Seven toggle chips, Monday first. */
  let { days = $bindable(), label }: { days: boolean[]; label: string } = $props()

  const DAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'] as const
</script>

<fieldset>
  <legend class="mb-1.5 font-medium">{label}</legend>
  <div class="flex flex-wrap gap-1.5">
    {#each DAYS as day, i (day)}
      <button
        type="button"
        aria-pressed={days[i]}
        aria-label={t(`settings.schedule.dayNamesLong.${day}`)}
        onclick={() => (days[i] = !days[i])}
        class="h-8 w-12 rounded-control border text-small font-medium transition-colors
          {days[i] ? 'border-fg bg-fg text-bg' : 'border-line bg-sunken text-fg-muted hover:border-line-strong'}"
      >
        {t(`settings.schedule.dayNames.${day}`)}
      </button>
    {/each}
  </div>
</fieldset>
