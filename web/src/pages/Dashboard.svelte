<script lang="ts">
  import type { Package } from '$lib/api'
  import ActivePackage from '$lib/components/dashboard/ActivePackage.svelte'
  import Attention from '$lib/components/dashboard/Attention.svelte'
  import RecentList from '$lib/components/dashboard/RecentList.svelte'
  import Resources from '$lib/components/dashboard/Resources.svelte'
  import SectionHeading from '$lib/components/dashboard/SectionHeading.svelte'
  import StatusHeader from '$lib/components/dashboard/StatusHeader.svelte'
  import UpNext from '$lib/components/dashboard/UpNext.svelte'
  import Welcome from '$lib/components/dashboard/Welcome.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { queue } from '$lib/stores/queue.svelte'
  import { isMovingPackage } from '$lib/utils/packages'

  let recent = $state<Package[]>([])
  const moving = $derived(queue.packages.filter(isMovingPackage))
  const hasQueue = $derived(queue.packages.length > 0)
</script>

<h1 class="sr-only">{t('dashboard.title')}</h1>

<StatusHeader />

<div class="mt-8 space-y-10">
  <Attention />

  {#if moving.length}
    <section aria-labelledby="current">
      <SectionHeading id="current" title={t('dashboard.current')} count={moving.length} />
      <ul class="space-y-3">
        {#each moving as pkg (pkg.id)}
          <ActivePackage {pkg} />
        {/each}
      </ul>
    </section>
  {/if}

  <Welcome hasHistory={recent.length > 0} />

  <div class="grid gap-x-12 gap-y-10 {hasQueue ? 'xl:grid-cols-2' : ''}" class:hidden={!hasQueue && !recent.length}>
    {#if hasQueue}
      <UpNext />
    {/if}
    <RecentList bind:items={recent} hideEmpty={!hasQueue} />
  </div>

  <Resources />
</div>
