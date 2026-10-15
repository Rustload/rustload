<script lang="ts">
  import type { Component } from 'svelte'
  import { SETTINGS_SECTIONS, type SettingsSectionId } from '$lib/components/layout/nav'
  import Button from '$lib/components/ui/Button.svelte'
  import Loading from '$lib/components/ui/Loading.svelte'
  import PageHeader from '$lib/components/ui/PageHeader.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { router } from '$lib/router.svelte'
  import { settings } from '$lib/stores/settings.svelte'
  import About from './settings/About.svelte'
  import Accounts from './settings/Accounts.svelte'
  import Downloads from './settings/Downloads.svelte'
  import Folders from './settings/Folders.svelte'
  import General from './settings/General.svelte'
  import Integrations from './settings/Integrations.svelte'
  import Media from './settings/Media.svelte'
  import Notifications from './settings/Notifications.svelte'
  import PostProcessing from './settings/PostProcessing.svelte'
  import Schedule from './settings/Schedule.svelte'
  import Security from './settings/Security.svelte'
  import System from './settings/System.svelte'

  let { section }: { section?: string } = $props()

  const VIEWS: Record<SettingsSectionId, Component> = {
    general: General,
    downloads: Downloads,
    schedule: Schedule,
    folders: Folders,
    postprocess: PostProcessing,
    accounts: Accounts,
    media: Media,
    notifications: Notifications,
    integrations: Integrations,
    system: System,
    security: Security,
    about: About,
  }

  const current = $derived(SETTINGS_SECTIONS.find((s) => s.id === section) ?? SETTINGS_SECTIONS[0])
  const View = $derived(VIEWS[current.id])
</script>

<PageHeader title={t('settings.title')} />

<div class="grid gap-6 lg:grid-cols-[13rem_minmax(0,1fr)] lg:gap-10">
  <nav aria-label={t('settings.sectionsLabel')} class="-mx-4 overflow-x-auto px-4 lg:mx-0 lg:overflow-visible lg:px-0">
    <ul class="flex gap-1 lg:sticky lg:top-20 lg:flex-col">
      {#each SETTINGS_SECTIONS as item (item.id)}
        {@const active = item.id === current.id}
        <li class="shrink-0">
          <a
            href={router.href(`/settings/${item.id}`)}
            aria-current={active ? 'page' : undefined}
            class="flex h-8 items-center gap-2.5 rounded-control px-3 text-small font-medium whitespace-nowrap transition-colors
              {active ? 'bg-sunken text-fg' : 'text-fg-muted hover:text-fg'}"
          >
            <item.icon size={15} aria-hidden="true" />
            {t(`settings.sections.${item.id}`)}
          </a>
        </li>
      {/each}
    </ul>
  </nav>

  <div class="max-w-3xl min-w-0 space-y-6">
    <h2 class="heading text-lg font-semibold">{t(`settings.sections.${current.id}`)}</h2>
    {#if settings.draft}
      <View />
    {:else}
      <Loading />
    {/if}
  </div>
</div>

{#if settings.dirty}
  <div
    class="fixed inset-x-3 bottom-[4.5rem] z-40 mx-auto flex max-w-2xl flex-wrap items-center gap-3 rounded-panel border border-line
      bg-raised py-2.5 pr-2.5 pl-4 shadow-pop lg:bottom-5 lg:left-[calc(15rem+1.5rem)]"
    role="region"
    aria-label={t('common.unsaved')}
  >
    <p class="min-w-0 flex-1 font-medium">{t('common.unsaved')}</p>
    {#if settings.error}
      <p class="w-full text-small text-bad sm:order-last">{settings.error}</p>
    {/if}
    <Button size="sm" onclick={() => settings.reset()}>{t('common.reset')}</Button>
    <Button size="sm" variant="primary" loading={settings.saving} onclick={() => settings.save()}>
      {t('common.saveChanges')}
    </Button>
  </div>
{/if}
