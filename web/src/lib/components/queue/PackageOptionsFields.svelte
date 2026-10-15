<script lang="ts" module>
  export type PackageOptions = {
    category_id: number | null
    create_subfolder: boolean
    extract: boolean
    delete_archives: boolean
    archive_password: string
  }
</script>

<script lang="ts">
  import { t } from '$lib/i18n/index.svelte'
  import { categories } from '$lib/stores/catalog.svelte'
  import Field from '../ui/Field.svelte'
  import Select from '../ui/Select.svelte'
  import TextInput from '../ui/TextInput.svelte'
  import Toggle from '../ui/Toggle.svelte'

  /** Destination and post-processing options shared by the add and edit dialogs. */
  type Props = {
    options: PackageOptions
    /** Destination fields are read-only once a file started downloading. */
    destinationLocked?: boolean
    destinationError?: string | null
  }

  let { options = $bindable(), destinationLocked = false, destinationError = null }: Props = $props()

  const categoryOptions = $derived([
    { value: null as number | null, label: t('add.automatic') },
    ...categories.list.map((c) => ({ value: c.id as number | null, label: `${c.name} (${c.resolved_path})` })),
  ])
</script>

<div class="space-y-4">
  <Field
    label={t('add.destination')}
    error={destinationError}
    hint={destinationLocked ? t('queue.destLocked') : undefined}
  >
    {#snippet children({ id, describedBy })}
      <Select
        {id}
        {describedBy}
        bind:value={options.category_id}
        options={categoryOptions}
        disabled={destinationLocked}
      />
    {/snippet}
  </Field>

  <div class="space-y-3.5 rounded-control border border-line px-4 py-3.5">
    <Toggle bind:checked={options.create_subfolder} label={t('add.subfolder')} disabled={destinationLocked} />
    <Toggle bind:checked={options.extract} label={t('add.extract')} />
    <Toggle bind:checked={options.delete_archives} label={t('add.deleteArchives')} disabled={!options.extract} />
  </div>

  {#if options.extract}
    <Field label={t('add.password')} optional>
      {#snippet children({ id })}
        <TextInput {id} bind:value={options.archive_password} autocomplete="off" spellcheck={false} />
      {/snippet}
    </Field>
  {/if}
</div>
