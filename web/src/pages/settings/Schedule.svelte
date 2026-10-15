<script lang="ts">
  import Plus from '@lucide/svelte/icons/plus'
  import Trash2 from '@lucide/svelte/icons/trash-2'
  import type { SpeedRule } from '$lib/api'
  import DayPicker from '$lib/components/settings/DayPicker.svelte'
  import DayTimeline from '$lib/components/settings/DayTimeline.svelte'
  import Button from '$lib/components/ui/Button.svelte'
  import Field from '$lib/components/ui/Field.svelte'
  import IconButton from '$lib/components/ui/IconButton.svelte'
  import NumberInput from '$lib/components/ui/NumberInput.svelte'
  import Panel from '$lib/components/ui/Panel.svelte'
  import PanelEmpty from '$lib/components/ui/PanelEmpty.svelte'
  import TimeInput from '$lib/components/ui/TimeInput.svelte'
  import Toggle from '$lib/components/ui/Toggle.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { formatSpeed, kibToBytes } from '$lib/utils/format'

  const MAX_RULES = 10

  const s = $derived(settings.draft!.schedule)
  const overnight = $derived(s.end < s.start)

  function addRule() {
    const rule: SpeedRule = {
      enabled: true,
      start: '08:00',
      end: '19:00',
      days: [true, true, true, true, true, false, false],
      limit_kib: 5 * 1024,
    }
    s.speed_rules = [...s.speed_rules, rule]
  }

  function removeRule(index: number) {
    s.speed_rules = s.speed_rules.filter((_, i) => i !== index)
  }
</script>

<Panel>
  <Toggle bind:checked={s.enabled} label={t('settings.schedule.enabled')} hint={t('settings.schedule.enabledHint')} />

  {#if s.enabled}
    <div class="grid max-w-md grid-cols-2 gap-4">
      <Field label={t('settings.schedule.start')}>
        {#snippet children({ id })}
          <TimeInput {id} bind:value={s.start} required />
        {/snippet}
      </Field>
      <Field label={t('settings.schedule.end')}>
        {#snippet children({ id })}
          <TimeInput {id} bind:value={s.end} required />
        {/snippet}
      </Field>
    </div>

    <DayPicker bind:days={s.days} label={t('settings.schedule.days')} />

    <figure>
      <DayTimeline start={s.start} end={s.end} label={t('settings.schedule.timeline')} />
      <figcaption class="mt-1.5 flex justify-between num text-xs text-fg-faint">
        <span>00:00</span><span>06:00</span><span>12:00</span><span>18:00</span><span>24:00</span>
      </figcaption>
    </figure>

    {#if overnight}
      <p class="text-small text-fg-muted">{t('settings.schedule.overnight')}</p>
    {/if}
  {:else}
    <p class="text-small text-fg-muted">{t('settings.schedule.always')}</p>
  {/if}
</Panel>

<Panel title={t('settings.schedule.rules')} description={t('settings.schedule.rulesHint')} flush>
  {#snippet actions()}
    <Button size="sm" icon={Plus} onclick={addRule} disabled={s.speed_rules.length >= MAX_RULES}>
      {t('settings.schedule.addRule')}
    </Button>
  {/snippet}

  {#if s.speed_rules.length === 0}
    <PanelEmpty text={t('settings.schedule.noRules')} />
  {:else}
    <ul class="divide-y divide-line border-t border-line">
      {#each s.speed_rules as rule, i (i)}
        <li class="space-y-4 px-5 py-4 {rule.enabled ? '' : 'opacity-70'}">
          <div class="flex items-center gap-3">
            <Toggle bind:checked={rule.enabled} ariaLabel={t('settings.schedule.ruleEnabled')} />
            <p class="min-w-0 flex-1 font-medium">
              {t('settings.schedule.ruleSummary', {
                speed: formatSpeed(kibToBytes(rule.limit_kib)),
                start: rule.start,
                end: rule.end,
              })}
            </p>
            <IconButton icon={Trash2} label={t('common.remove')} size="sm" onclick={() => removeRule(i)} />
          </div>
          <div class="grid gap-4 sm:grid-cols-3">
            <Field label={t('settings.schedule.start')}>
              {#snippet children({ id })}
                <TimeInput {id} bind:value={rule.start} required />
              {/snippet}
            </Field>
            <Field label={t('settings.schedule.end')}>
              {#snippet children({ id })}
                <TimeInput {id} bind:value={rule.end} required />
              {/snippet}
            </Field>
            <Field label={t('settings.schedule.ruleLimit')}>
              {#snippet children({ id, describedBy })}
                <NumberInput
                  {id}
                  {describedBy}
                  bind:value={rule.limit_kib}
                  min={1}
                  suffix={t('settings.downloads.kibSuffix')}
                />
              {/snippet}
            </Field>
          </div>
          <DayPicker bind:days={rule.days} label={t('settings.schedule.days')} />
          <DayTimeline
            start={rule.start}
            end={rule.end}
            tone="muted"
            compact
            label={t('settings.schedule.ruleTimeline')}
          />
        </li>
      {/each}
    </ul>
  {/if}
</Panel>
