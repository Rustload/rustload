<script lang="ts">
  import { untrack } from 'svelte'
  import { ApiError, api, type MediaOptions, type Package, type PackagePatch } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { errorMessage } from '$lib/stores/toasts.svelte'
  import MediaOptionsFields from '../add/MediaOptionsFields.svelte'
  import Alert from '../ui/Alert.svelte'
  import Button from '../ui/Button.svelte'
  import Dialog from '../ui/Dialog.svelte'
  import Field from '../ui/Field.svelte'
  import TextInput from '../ui/TextInput.svelte'
  import PackageOptionsFields, { type PackageOptions } from './PackageOptionsFields.svelte'

  let { pkg, open = $bindable() }: { pkg: Package; open: boolean } = $props()

  let name = $state('')
  let options = $state<PackageOptions>({
    category_id: null,
    create_subfolder: true,
    extract: true,
    delete_archives: true,
    archive_password: '',
  })
  let media = $state<MediaOptions | null>(null)
  let saving = $state(false)
  let error = $state<string | null>(null)
  let destinationError = $state<string | null>(null)

  const locked = $derived(pkg.dest_dir !== null)

  // Fill the form each time the dialog opens; live queue refreshes must not reset it while editing.
  $effect(() => {
    if (!open) return
    untrack(() => {
      name = pkg.name
      options = {
        category_id: pkg.category_id,
        create_subfolder: pkg.create_subfolder,
        extract: pkg.extract,
        delete_archives: pkg.delete_archives,
        archive_password: pkg.archive_password ?? '',
      }
      media = pkg.media ? structuredClone($state.snapshot(pkg.media)) : null
      error = null
      destinationError = null
    })
  })

  function changes(): PackagePatch {
    const patch: PackagePatch = {}
    if (name.trim() && name.trim() !== pkg.name) patch.name = name.trim()
    if (!locked && options.category_id !== pkg.category_id) patch.category_id = options.category_id
    if (!locked && options.create_subfolder !== pkg.create_subfolder) patch.create_subfolder = options.create_subfolder
    if (options.extract !== pkg.extract) patch.extract = options.extract
    if (options.delete_archives !== pkg.delete_archives) patch.delete_archives = options.delete_archives
    const password = options.archive_password.trim() || null
    if (password !== pkg.archive_password) patch.archive_password = password
    if (media && JSON.stringify(media) !== JSON.stringify(pkg.media)) patch.media = $state.snapshot(media)
    return patch
  }

  async function save(event: SubmitEvent) {
    event.preventDefault()
    const patch = changes()
    if (Object.keys(patch).length === 0) {
      open = false
      return
    }
    saving = true
    error = null
    destinationError = null
    try {
      await api.packages.update(pkg.id, patch)
      queue.invalidate()
      open = false
    } catch (err) {
      if (err instanceof ApiError && err.code === 'DEST_LOCKED') destinationError = t('queue.destLocked')
      else error = errorMessage(err)
    } finally {
      saving = false
    }
  }
</script>

<Dialog bind:open title={t('queue.editTitle')} initialFocus="input">
  <form id="edit-package-{pkg.id}" class="space-y-4" onsubmit={save}>
    <Field label={t('add.name')}>
      {#snippet children({ id })}
        <TextInput {id} bind:value={name} required />
      {/snippet}
    </Field>
    <PackageOptionsFields bind:options destinationLocked={locked} {destinationError} />
    {#if media}
      <section aria-labelledby="edit-media-{pkg.id}" class="space-y-3 border-t border-line pt-4">
        <h3 id="edit-media-{pkg.id}" class="heading font-semibold">{t('media.title')}</h3>
        <MediaOptionsFields bind:options={media} />
      </section>
    {/if}
    {#if error}
      <Alert>{error}</Alert>
    {/if}
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)}>{t('common.cancel')}</Button>
    <Button type="submit" form="edit-package-{pkg.id}" variant="primary" loading={saving}
      >{t('common.saveChanges')}</Button
    >
  {/snippet}
</Dialog>
