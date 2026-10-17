<script lang="ts">
  import ArrowDown from '@lucide/svelte/icons/arrow-down'
  import ArrowUp from '@lucide/svelte/icons/arrow-up'
  import Pencil from '@lucide/svelte/icons/pencil'
  import Plus from '@lucide/svelte/icons/plus'
  import Star from '@lucide/svelte/icons/star'
  import Trash2 from '@lucide/svelte/icons/trash-2'
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert'
  import type { Category } from '$lib/api'
  import CategoryDialog from '$lib/components/settings/CategoryDialog.svelte'
  import Badge from '$lib/components/ui/Badge.svelte'
  import Button from '$lib/components/ui/Button.svelte'
  import Field from '$lib/components/ui/Field.svelte'
  import Menu from '$lib/components/ui/Menu.svelte'
  import NumberInput from '$lib/components/ui/NumberInput.svelte'
  import Panel from '$lib/components/ui/Panel.svelte'
  import Toggle from '$lib/components/ui/Toggle.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { auth } from '$lib/stores/auth.svelte'
  import { categories } from '$lib/stores/catalog.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import { runWithToast } from '$lib/stores/toasts.svelte'
  import { system } from '$lib/stores/system.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { formatBytes } from '$lib/utils/format'

  const storage = $derived(settings.draft!.storage)

  let editing = $state<Category | null>(null)
  let dialogOpen = $state(false)

  function open(category: Category | null) {
    editing = category
    dialogOpen = true
  }

  function makeDefault(category: Category) {
    runWithToast(() =>
      categories.save(category.id, {
        name: category.name,
        path: category.path,
        extensions: category.extensions,
        pattern: category.pattern,
        is_default: true,
      }),
    )
  }

  async function remove(category: Category) {
    const { confirmed } = await ui.confirm({
      title: t('settings.folders.deleteTitle', { name: category.name }),
      body: t('settings.folders.deleteBody'),
      confirmLabel: t('common.delete'),
      danger: true,
    })
    if (confirmed) runWithToast(() => categories.remove(category.id))
  }
</script>

<Panel
  title={t('settings.folders.root')}
  description={auth.desktop ? t('settings.folders.rootHintApp') : t('settings.folders.rootHint')}
>
  <p class="rounded-control bg-sunken px-3 py-2 font-mono text-small break-all">{categories.root || '—'}</p>
  <Toggle
    bind:checked={storage.create_subfolder}
    label={t('settings.folders.subfolder')}
    hint={t('settings.folders.subfolderHint')}
  />
</Panel>

<Panel title={t('settings.folders.categories')} description={t('settings.folders.categoriesHint')} flush>
  {#snippet actions()}
    <Button size="sm" icon={Plus} onclick={() => open(null)}>{t('settings.folders.add')}</Button>
  {/snippet}
  <ul class="divide-y divide-line border-t border-line">
    {#each categories.list as category, i (category.id)}
      <li class="flex items-center gap-3 py-3 pr-3 pl-5">
        <div class="min-w-0 flex-1">
          <p class="flex items-center gap-2 font-medium">
            {category.name}
            {#if category.is_default}
              <Badge tone="muted" icon={Star} label={t('settings.folders.default')} />
            {/if}
          </p>
          <p class="mt-0.5 truncate font-mono text-xs text-fg-muted" title={category.resolved_path}>
            {category.resolved_path}
          </p>
          {#if category.missing}
            <p class="mt-1 flex items-start gap-1.5 text-small text-warn">
              <TriangleAlert size={14} class="mt-0.5 shrink-0" aria-hidden="true" />
              {auth.desktop ? t('settings.folders.missingApp') : t('settings.folders.missing')}
            </p>
          {/if}
          <p class="mt-1.5 flex flex-wrap gap-1">
            {#if category.pattern}
              <span class="rounded-chip bg-sunken px-1.5 text-xs text-fg-muted">
                {t('settings.folders.ruleChip')}
                <span class="font-mono text-fg">{category.pattern}</span>
              </span>
            {/if}
            {#each category.extensions.slice(0, 10) as ext (ext)}
              <span class="rounded-chip bg-sunken px-1.5 num text-xs text-fg-muted">{ext}</span>
            {/each}
            {#if category.extensions.length > 10}
              <span class="px-1 num text-xs text-fg-faint">+{category.extensions.length - 10}</span>
            {/if}
            {#if !category.pattern && !category.extensions.length}
              <span class="text-xs text-fg-faint">
                {category.is_default ? t('settings.folders.catchAll') : t('settings.folders.manualOnly')}
              </span>
            {/if}
          </p>
        </div>
        <Menu
          items={[
            { label: t('common.edit'), icon: Pencil, onselect: () => open(category) },
            {
              label: t('settings.folders.makeDefault'),
              icon: Star,
              disabled: category.is_default,
              onselect: () => makeDefault(category),
            },
            {
              label: t('queue.moveUp'),
              icon: ArrowUp,
              disabled: i === 0,
              onselect: () => runWithToast(() => categories.move(i, -1)),
            },
            {
              label: t('queue.moveDown'),
              icon: ArrowDown,
              disabled: i === categories.list.length - 1,
              onselect: () => runWithToast(() => categories.move(i, 1)),
            },
            'separator',
            {
              label: t('common.delete'),
              icon: Trash2,
              danger: true,
              disabled: category.is_default,
              onselect: () => remove(category),
            },
          ]}
        />
      </li>
    {/each}
  </ul>
</Panel>

<CategoryDialog category={editing} bind:open={dialogOpen} />

<Panel title={t('settings.folders.guard')} description={t('settings.folders.guardHint')}>
  <Field
    label={t('settings.folders.minFree')}
    hint={system.info?.disk
      ? t('settings.folders.freeNow', { free: formatBytes(system.info.disk.free) })
      : t('settings.folders.minFreeOff')}
  >
    {#snippet children({ id, describedBy })}
      <NumberInput
        {id}
        {describedBy}
        bind:value={() => storage.min_free_mib / 1024, (gib) => (storage.min_free_mib = Math.round(gib * 1024))}
        min={0}
        step={0.5}
        suffix={t('units.gib')}
        class="max-w-48"
      />
    {/snippet}
  </Field>
</Panel>
