<script lang="ts">
  import Play from '@lucide/svelte/icons/play'
  import { api } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { router } from '$lib/router.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import { formatBytes, formatPercent } from '$lib/utils/format'
  import { isMovingPackage, isPausedPackage, packageProgress } from '$lib/utils/packages'
  import IconButton from '../ui/IconButton.svelte'
  import SectionHeading from './SectionHeading.svelte'

  const SHOWN = 5

  /** Packages that start next, in queue order; paused ones stay in line and say so. */
  const waiting = $derived(
    queue.packages.filter((p) => !isMovingPackage(p) && p.status !== 'failed' && p.status !== 'completed'),
  )
</script>

<section aria-labelledby="up-next">
  <SectionHeading id="up-next" title={t('dashboard.upNext')} count={waiting.length} />
  {#if waiting.length}
    <ul class="divide-y divide-line border-y border-line">
      {#each waiting.slice(0, SHOWN) as pkg (pkg.id)}
        {@const progress = packageProgress(pkg)}
        {@const paused = isPausedPackage(pkg)}
        <li class="flex items-center gap-2">
          <button
            type="button"
            onclick={() => ui.revealPackage(pkg.id)}
            class="flex min-w-0 flex-1 items-center gap-4 py-2 text-left hover:bg-sunken/50"
          >
            <span class="min-w-0 flex-1">
              <span class="block truncate text-small">{pkg.name}</span>
              <span class="flex gap-3 text-xs text-fg-faint">
                <span>{t('dashboard.card.files', { count: pkg.files.length })}</span>
                {#if paused}<span class="text-fg-muted">{t('dashboard.paused')}</span>{/if}
                {#if progress.downloaded > 0}<span class="num">{formatPercent(progress.ratio)}</span>{/if}
              </span>
            </span>
            <span class="shrink-0 num text-xs text-fg-muted">{formatBytes(progress.total)}</span>
          </button>
          {#if paused}
            <IconButton
              icon={Play}
              label={t('dashboard.resumePackage', { name: pkg.name })}
              size="sm"
              onclick={() => queue.run(() => api.packages.resume(pkg.id))}
            />
          {/if}
        </li>
      {/each}
    </ul>
    {#if waiting.length > SHOWN}
      <a
        href={router.href('/queue')}
        class="mt-2 inline-block text-small text-fg-muted underline-offset-2 hover:underline"
      >
        {t('dashboard.morePackages', { count: waiting.length - SHOWN })}
      </a>
    {/if}
  {:else}
    <p class="border-y border-line py-3 text-small text-fg-faint">{t('dashboard.nothingWaiting')}</p>
  {/if}
</section>
