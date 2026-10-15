<script lang="ts">
  import { t } from '$lib/i18n/index.svelte'
  import Spinner from './Spinner.svelte'

  /** Spinner standing in for a page (or, `compact`, a panel) until its data arrives. */
  let { compact = false }: { compact?: boolean } = $props()
</script>

<div role="status" class="grid place-items-center text-fg-faint {compact ? 'py-8' : 'py-16'}">
  <Spinner size={compact ? 18 : 20} />
  <span class="sr-only">{t('common.loading')}</span>
</div>
