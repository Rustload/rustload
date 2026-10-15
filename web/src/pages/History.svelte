<script lang="ts">
  import ChevronLeft from '@lucide/svelte/icons/chevron-left'
  import ChevronRight from '@lucide/svelte/icons/chevron-right'
  import HistoryIcon from '@lucide/svelte/icons/history'
  import Trash2 from '@lucide/svelte/icons/trash-2'
  import { onMount } from 'svelte'
  import { clearHistory } from '$lib/actions'
  import HistoryRow from '$lib/components/history/HistoryRow.svelte'
  import Button from '$lib/components/ui/Button.svelte'
  import EmptyState from '$lib/components/ui/EmptyState.svelte'
  import IconButton from '$lib/components/ui/IconButton.svelte'
  import Loading from '$lib/components/ui/Loading.svelte'
  import PageHeader from '$lib/components/ui/PageHeader.svelte'
  import SearchInput from '$lib/components/ui/SearchInput.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { history } from '$lib/stores/history.svelte'

  onMount(() => history.attach())

  let query = $state(history.query)
</script>

<PageHeader title={t('history.title')} count={history.loaded ? t('history.count', { count: history.total }) : null}>
  {#snippet actions()}
    <div class="flex w-full items-center gap-2 sm:w-auto">
      <SearchInput
        bind:value={query}
        oninput={() => history.search(query)}
        placeholder={t('history.searchPlaceholder')}
        label={t('common.search')}
        class="flex-1 sm:w-72"
      />
      <Button icon={Trash2} onclick={clearHistory} disabled={history.total === 0}>{t('history.clear')}</Button>
    </div>
  {/snippet}
</PageHeader>

{#if !history.loaded}
  <Loading />
{:else if history.items.length === 0}
  {#if history.query}
    <p class="py-12 text-center text-fg-muted">{t('history.noResults', { query: history.query })}</p>
  {:else}
    <EmptyState icon={HistoryIcon} title={t('history.emptyTitle')} body={t('history.emptyBody')} />
  {/if}
{:else}
  <ul class="rounded-panel border border-line bg-panel px-3 {history.loading ? 'opacity-70' : ''}">
    {#each history.items as pkg (pkg.id)}
      <HistoryRow {pkg} />
    {/each}
  </ul>

  {#if history.pages > 1}
    <nav
      class="mt-4 flex items-center justify-end gap-2"
      aria-label={t('history.page', { page: history.page + 1, pages: history.pages })}
    >
      <span class="mr-2 num text-small text-fg-muted">
        {t('history.page', { page: history.page + 1, pages: history.pages })}
      </span>
      <IconButton
        icon={ChevronLeft}
        label={t('history.previous')}
        variant="secondary"
        size="sm"
        disabled={history.page === 0}
        onclick={() => history.goTo(history.page - 1)}
      />
      <IconButton
        icon={ChevronRight}
        label={t('history.next')}
        variant="secondary"
        size="sm"
        disabled={history.page >= history.pages - 1}
        onclick={() => history.goTo(history.page + 1)}
      />
    </nav>
  {/if}
{/if}
