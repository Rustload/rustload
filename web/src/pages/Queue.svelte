<script lang="ts">
  import ListOrdered from '@lucide/svelte/icons/list-ordered'
  import Pause from '@lucide/svelte/icons/pause'
  import Play from '@lucide/svelte/icons/play'
  import Plus from '@lucide/svelte/icons/plus'
  import RotateCcw from '@lucide/svelte/icons/rotate-ccw'
  import Trash2 from '@lucide/svelte/icons/trash-2'
  import X from '@lucide/svelte/icons/x'
  import { tick } from 'svelte'
  import { SvelteSet } from 'svelte/reactivity'
  import { deletePackages } from '$lib/actions'
  import { api, type Package } from '$lib/api'
  import EditPackageDialog from '$lib/components/queue/EditPackageDialog.svelte'
  import PackageRow from '$lib/components/queue/PackageRow.svelte'
  import Button from '$lib/components/ui/Button.svelte'
  import EmptyState from '$lib/components/ui/EmptyState.svelte'
  import Loading from '$lib/components/ui/Loading.svelte'
  import PageHeader from '$lib/components/ui/PageHeader.svelte'
  import SearchInput from '$lib/components/ui/SearchInput.svelte'
  import Segmented from '$lib/components/ui/Segmented.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { isMovingPackage, isPausedPackage } from '$lib/utils/packages'

  const selection = new SvelteSet<number>()
  let editing = $state<Package | null>(null)
  let editOpen = $state(false)

  type Filter = 'all' | 'active' | 'waiting' | 'paused' | 'failed'
  let filter = $state<Filter>('all')
  let search = $state('')
  const expanded = new SvelteSet<number>()
  let highlighted = $state<number | null>(null)

  // Active, waiting and paused don't overlap: active matches the dashboard's current packages, waiting and
  // paused its "up next" list.
  const MATCH: Record<Filter, (p: Package) => boolean> = {
    all: () => true,
    active: isMovingPackage,
    waiting: (p) => p.status === 'queued' && !isPausedPackage(p) && !isMovingPackage(p),
    paused: (p) => isPausedPackage(p) && !isMovingPackage(p),
    failed: (p) => p.status === 'failed',
  }
  const counts = $derived(
    Object.fromEntries(
      (Object.keys(MATCH) as Filter[]).map((f) => [f, queue.packages.filter(MATCH[f]).length]),
    ) as Record<Filter, number>,
  )
  const needle = $derived(search.trim().toLowerCase())
  const visible = $derived(
    queue.packages.filter(
      (p) =>
        MATCH[filter](p) &&
        (!needle ||
          p.name.toLowerCase().includes(needle) ||
          p.files.some((f) => (f.filename ?? f.url).toLowerCase().includes(needle))),
    ),
  )
  // Reordering only makes sense on the full, unfiltered list.
  const reorderable = $derived(filter === 'all' && !needle)

  // A package opened from the command palette: show it, unfold it and outline it for a moment.
  let highlightTimer: ReturnType<typeof setTimeout> | undefined
  $effect(() => {
    const id = ui.focusPackage
    if (id === null || !queue.loaded) return
    ui.focusPackage = null
    filter = 'all'
    search = ''
    expanded.add(id)
    highlighted = id
    const behavior = matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth'
    tick().then(() => document.getElementById(`package-${id}`)?.scrollIntoView({ block: 'center', behavior }))
    clearTimeout(highlightTimer)
    highlightTimer = setTimeout(() => (highlighted = null), 2200)
  })
  $effect(() => () => clearTimeout(highlightTimer))

  // Drop selected ids that left the queue.
  $effect(() => {
    const ids = new Set(queue.packages.map((p) => p.id))
    for (const id of selection) if (!ids.has(id)) selection.delete(id)
  })

  // Selected packages hidden by the filter or the search are left alone by bulk actions.
  const selected = $derived(visible.filter((p) => selection.has(p.id)))
  const allSelected = $derived(visible.length > 0 && visible.every((p) => selection.has(p.id)))

  function toggleAll(checked: boolean) {
    selection.clear()
    if (checked) for (const p of visible) selection.add(p.id)
  }

  function bulk(action: (id: number) => Promise<unknown>) {
    const ids = selected.map((p) => p.id)
    queue.run(() => Promise.all(ids.map(action)))
  }

  async function bulkDelete() {
    if (await deletePackages(selected)) selection.clear()
  }

  function edit(pkg: Package) {
    editing = pkg
    editOpen = true
  }

  // ---- drag-and-drop reordering (keyboard users have the Move actions in each row menu) ----
  let dragId = $state<number | null>(null)
  let dropTarget = $state<{ id: number; after: boolean } | null>(null)

  function startDrag(pkg: Package, event: DragEvent) {
    dragId = pkg.id
    if (!event.dataTransfer) return
    event.dataTransfer.effectAllowed = 'move'
    event.dataTransfer.setData('text/plain', String(pkg.id))
    const row = (event.currentTarget as HTMLElement).closest('li')
    if (row) event.dataTransfer.setDragImage(row, 24, 20)
  }

  function dragOver(pkg: Package, event: DragEvent) {
    if (dragId === null) return
    event.preventDefault()
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
    dropTarget = { id: pkg.id, after: event.clientY > rect.top + rect.height / 2 }
  }

  function drop(event: DragEvent) {
    if (dragId === null) return
    event.preventDefault()
    if (dropTarget && dropTarget.id !== dragId) {
      const ids = queue.packages.map((p) => p.id).filter((id) => id !== dragId)
      const index = ids.indexOf(dropTarget.id) + (dropTarget.after ? 1 : 0)
      ids.splice(index, 0, dragId)
      queue.reorder(ids)
    }
    endDrag()
  }

  function endDrag() {
    dragId = null
    dropTarget = null
  }
</script>

<PageHeader title={t('queue.title')} count={queue.loaded ? t('queue.count', { count: queue.packages.length }) : null} />

{#if !queue.loaded}
  <Loading />
{:else if queue.packages.length === 0}
  <EmptyState icon={ListOrdered} title={t('queue.emptyTitle')} body={t('queue.emptyBody')}>
    {#snippet action()}
      <Button variant="primary" icon={Plus} onclick={() => ui.openAdd()}>{t('dashboard.emptyAction')}</Button>
    {/snippet}
  </EmptyState>
{:else}
  <div class="mb-3 flex flex-wrap items-center gap-2">
    <SearchInput bind:value={search} placeholder={t('queue.searchPlaceholder')} class="w-full sm:w-64" />
    <Segmented
      label={t('queue.filterLabel')}
      bind:value={filter}
      class="max-w-full overflow-x-auto"
      options={(['all', 'active', 'waiting', 'paused', 'failed'] as const)
        .filter((f) => f === 'all' || f === filter || counts[f] > 0)
        .map((f) => ({ value: f, label: t(`queue.filters.${f}`, { count: counts[f] }) }))}
    />
  </div>

  <div
    class="sticky top-14 z-20 -mx-2 mb-2 flex min-h-10 flex-wrap items-center gap-2 rounded-control px-2 py-1
      {selected.length ? 'bg-sunken shadow-[0_0_0_1px_var(--line)]' : 'bg-bg'}"
  >
    <label class="flex items-center gap-2.5 pr-2 pl-1.5 text-small text-fg-muted">
      <input
        type="checkbox"
        checked={allSelected}
        indeterminate={selected.length > 0 && !allSelected}
        onchange={(e) => toggleAll(e.currentTarget.checked)}
        class="size-4 cursor-pointer accent-[var(--fg)]"
      />
      {selected.length ? t('queue.selected', { count: selected.length }) : t('queue.selectAll')}
    </label>
    {#if selected.length}
      <Button size="sm" icon={Pause} onclick={() => bulk((id) => api.packages.pause(id))}>{t('common.pause')}</Button>
      <Button size="sm" icon={Play} onclick={() => bulk((id) => api.packages.resume(id))}>{t('common.resume')}</Button>
      <Button size="sm" icon={RotateCcw} onclick={() => bulk((id) => api.packages.retry(id))}
        >{t('common.retry')}</Button
      >
      <Button size="sm" icon={Trash2} onclick={bulkDelete}>{t('common.delete')}</Button>
      <Button size="sm" variant="ghost" icon={X} onclick={() => selection.clear()}>{t('queue.clearSelection')}</Button>
    {/if}
  </div>

  {#if visible.length === 0}
    <p class="py-10 text-center text-fg-muted">{t('queue.noMatch')}</p>
  {/if}

  <ul class="space-y-1.5" ondragend={endDrag}>
    {#each visible as pkg (pkg.id)}
      <li
        id="package-{pkg.id}"
        class="relative scroll-mt-28 {dragId === pkg.id ? 'opacity-40' : ''}"
        ondragover={(e) => dragOver(pkg, e)}
        ondrop={drop}
      >
        {#if dropTarget?.id === pkg.id && dragId !== pkg.id}
          <span
            class="absolute inset-x-0 z-10 h-0.5 rounded-full bg-info {dropTarget.after ? '-bottom-1' : '-top-1'}"
            aria-hidden="true"
          ></span>
        {/if}
        <PackageRow
          {pkg}
          first={queue.packages[0]?.id === pkg.id}
          last={queue.packages[queue.packages.length - 1]?.id === pkg.id}
          selected={selection.has(pkg.id)}
          onselect={(checked) => (checked ? selection.add(pkg.id) : selection.delete(pkg.id))}
          onedit={() => edit(pkg)}
          ondragstart={reorderable ? (e) => startDrag(pkg, e) : null}
          bind:expanded={() => expanded.has(pkg.id), (open) => (open ? expanded.add(pkg.id) : expanded.delete(pkg.id))}
          highlight={highlighted === pkg.id}
        />
      </li>
    {/each}
  </ul>
{/if}

{#if editing}
  <EditPackageDialog pkg={editing} bind:open={editOpen} />
{/if}
