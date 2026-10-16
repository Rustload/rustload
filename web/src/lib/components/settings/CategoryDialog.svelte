<script lang="ts">
  import { untrack } from 'svelte'
  import { ApiError, type Category } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { categories } from '$lib/stores/catalog.svelte'
  import { errorMessage } from '$lib/stores/toasts.svelte'
  import Alert from '../ui/Alert.svelte'
  import Button from '../ui/Button.svelte'
  import Dialog from '../ui/Dialog.svelte'
  import Field from '../ui/Field.svelte'
  import TagInput from '../ui/TagInput.svelte'
  import TextInput from '../ui/TextInput.svelte'
  import Toggle from '../ui/Toggle.svelte'

  /** `category` null creates a new one. */
  let { category, open = $bindable() }: { category: Category | null; open: boolean } = $props()

  let name = $state('')
  let path = $state('')
  let extensions = $state<string[]>([])
  let pattern = $state('')
  let isDefault = $state(false)
  let saving = $state(false)
  let error = $state<string | null>(null)
  let patternError = $state<string | null>(null)

  $effect(() => {
    if (!open) return
    untrack(() => {
      name = category?.name ?? ''
      path = category?.path ?? ''
      extensions = [...(category?.extensions ?? [])]
      pattern = category?.pattern ?? ''
      isDefault = category?.is_default ?? false
      error = null
      patternError = null
    })
  })

  function isAbsolute(p: string): boolean {
    return p.startsWith('/') || /^[a-z]:[\\/]/i.test(p)
  }

  const resolved = $derived.by(() => {
    const p = path.trim()
    if (isAbsolute(p)) return p
    const root = categories.root.replace(/[\\/]+$/, '')
    return p ? `${root}/${p.replace(/^[\\/]+/, '')}` : root
  })

  function normalizeExtension(raw: string): string | null {
    const ext = raw.trim().toLowerCase().replace(/^\.+/, '')
    return /^[a-z0-9]{1,10}$/.test(ext) ? ext : null
  }

  async function save(event: SubmitEvent) {
    event.preventDefault()
    if (!name.trim()) return
    saving = true
    error = null
    patternError = null
    try {
      await categories.save(category?.id ?? null, {
        name: name.trim(),
        path: path.trim(),
        extensions,
        pattern: pattern.trim(),
        is_default: isDefault,
      })
      open = false
    } catch (err) {
      if (err instanceof ApiError && err.code === 'INVALID_PATTERN') {
        patternError = t('settings.folders.ruleInvalid', { reason: err.message.replace(/^invalid rule: /, '') })
      } else {
        error = errorMessage(err)
      }
    } finally {
      saving = false
    }
  }
</script>

<Dialog bind:open title={category ? t('settings.folders.edit') : t('settings.folders.add')} initialFocus="input">
  <form id="category-form" class="space-y-4" onsubmit={save}>
    <Field label={t('settings.folders.name')}>
      {#snippet children({ id })}
        <TextInput {id} bind:value={name} required maxlength={60} />
      {/snippet}
    </Field>
    <Field label={t('settings.folders.path')} hint={t('settings.folders.pathHint')}>
      {#snippet children({ id, describedBy })}
        <TextInput {id} bind:value={path} mono spellcheck={false} autocomplete="off" aria-describedby={describedBy} />
        <p class="mt-1.5 truncate font-mono text-xs text-fg-faint" title={resolved}>
          {t('settings.folders.resolved', { path: resolved })}
        </p>
      {/snippet}
    </Field>
    <fieldset class="space-y-4">
      <legend class="font-medium">{t('settings.folders.automatic')}</legend>
      <p class="-mt-2 text-small text-fg-muted">{t('settings.folders.automaticHint')}</p>
      <Field label={t('settings.folders.rule')} hint={t('settings.folders.ruleHint')} error={patternError} optional>
        {#snippet children({ id, describedBy, invalid })}
          <TextInput
            {id}
            bind:value={pattern}
            {invalid}
            mono
            spellcheck={false}
            autocomplete="off"
            maxlength={500}
            placeholder={t('settings.folders.rulePlaceholder')}
            aria-describedby={describedBy}
            oninput={() => (patternError = null)}
          />
        {/snippet}
      </Field>
      <Field label={t('settings.folders.extensions')} hint={t('settings.folders.extensionsHint')} optional>
        {#snippet children({ id, describedBy })}
          <TagInput
            {id}
            {describedBy}
            bind:values={extensions}
            normalize={normalizeExtension}
            placeholder={t('settings.folders.extensionsPlaceholder')}
          />
        {/snippet}
      </Field>
    </fieldset>
    <Toggle
      bind:checked={isDefault}
      label={t('settings.folders.default')}
      hint={t('settings.folders.defaultHint')}
      disabled={category?.is_default}
    />
    {#if error}
      <Alert>{error}</Alert>
    {/if}
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)}>{t('common.cancel')}</Button>
    <Button type="submit" form="category-form" variant="primary" loading={saving} disabled={!name.trim()}>
      {t('common.save')}
    </Button>
  {/snippet}
</Dialog>
