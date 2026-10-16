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

<!-- Stacked in narrow dialogs; across a wide one (the add dialog), the fields on the left and the
     switches on the right. -->
<div class="@container">
  <div class="grid gap-x-8 gap-y-4 @2xl:grid-cols-2">
    <Field
      label={t('add.destination')}
      error={destinationError}
      hint={destinationLocked ? t('queue.destLocked') : undefined}
      class="@2xl:col-start-1 @2xl:row-start-1"
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

    <!-- Below the switches when stacked (it belongs to extraction), hidden while it is off; under
       the destination across, greyed out while extraction is off. -->
    <Field
      label={t('add.password')}
      optional
      class="order-last @2xl:col-start-1 @2xl:row-start-2 {options.extract ? '' : 'hidden @2xl:block'}"
    >
      {#snippet children({ id })}
        <TextInput
          {id}
          bind:value={options.archive_password}
          autocomplete="off"
          spellcheck={false}
          disabled={!options.extract}
        />
      {/snippet}
    </Field>

    <div
      class="space-y-3.5 self-start rounded-control border border-line px-4 py-3.5 @2xl:col-start-2 @2xl:row-span-2 @2xl:row-start-1"
    >
      <Toggle bind:checked={options.create_subfolder} label={t('add.subfolder')} disabled={destinationLocked} />
      <Toggle bind:checked={options.extract} label={t('add.extract')} />
      <Toggle bind:checked={options.delete_archives} label={t('add.deleteArchives')} disabled={!options.extract} />
    </div>
  </div>
</div>
