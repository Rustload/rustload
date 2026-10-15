<script lang="ts">
  import X from '@lucide/svelte/icons/x'
  import { t } from '$lib/i18n/index.svelte'
  import IconButton from './IconButton.svelte'

  type Props = {
    values: string[]
    id?: string
    describedBy?: string
    placeholder?: string
    /** Normalises a typed value; return null to reject it. */
    normalize?: (raw: string) => string | null
  }

  let { values = $bindable(), id, describedBy, placeholder, normalize = (raw) => raw.trim() || null }: Props = $props()

  let draft = $state('')

  function commit() {
    const additions = draft
      .split(/[\s,;]+/)
      .map((raw) => normalize(raw))
      .filter((v): v is string => !!v && !values.includes(v))
    if (additions.length) values = [...values, ...new Set(additions)]
    draft = ''
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' || event.key === ',' || event.key === ' ') {
      if (draft.trim()) {
        event.preventDefault()
        commit()
      }
    } else if (event.key === 'Backspace' && draft === '' && values.length) {
      values = values.slice(0, -1)
    }
  }
</script>

<div
  class="flex min-h-9 w-full flex-wrap items-center gap-1.5 rounded-control border border-line bg-sunken px-2 py-1.5
    transition-colors focus-within:border-info focus-within:ring-2 focus-within:ring-info/25 hover:border-line-strong"
>
  {#each values as value (value)}
    <span class="inline-flex h-6 items-center gap-1 rounded-chip border border-line bg-raised pr-1 pl-2 num text-small">
      {value}
      <IconButton
        icon={X}
        label={`${t('common.remove')} ${value}`}
        size="xs"
        onclick={() => (values = values.filter((v) => v !== value))}
      />
    </span>
  {/each}
  <input
    {id}
    bind:value={draft}
    {onkeydown}
    onblur={commit}
    aria-describedby={describedBy}
    placeholder={values.length ? '' : placeholder}
    class="h-6 min-w-24 flex-1 bg-transparent text-sm outline-none placeholder:text-fg-faint focus-visible:outline-none"
  />
</div>
