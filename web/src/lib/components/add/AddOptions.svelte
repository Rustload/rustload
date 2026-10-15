<script lang="ts">
  import ChevronDown from '@lucide/svelte/icons/chevron-down'
  import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal'
  import type { MediaOptions } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { categories } from '$lib/stores/catalog.svelte'
  import PackageOptionsFields, { type PackageOptions } from '../queue/PackageOptionsFields.svelte'
  import MediaOptionsFields from './MediaOptionsFields.svelte'

  /** The grabber's options, folded into one line that says what will happen. */
  type Props = {
    options: PackageOptions
    media: MediaOptions
    /** Video and audio options are only shown when a link goes through yt-dlp. */
    hasMedia: boolean
  }

  let { options = $bindable(), media = $bindable(), hasMedia }: Props = $props()

  let open = $state(false)
  const id = $props.id()

  const summary = $derived.by(() => {
    const category = categories.list.find((c) => c.id === options.category_id)
    const parts = [category ? category.name : t('grabber.summary.automatic')]
    if (options.create_subfolder) parts.push(t('grabber.summary.subfolder'))
    if (!options.extract) parts.push(t('grabber.summary.noExtract'))
    else parts.push(t(options.delete_archives ? 'grabber.summary.extractDelete' : 'grabber.summary.extract'))
    if (options.extract && options.archive_password.trim()) parts.push(t('grabber.summary.password'))
    if (hasMedia) {
      parts.push(
        media.quality === 'audio'
          ? t('grabber.summary.audio')
          : media.quality === 'best'
            ? t('grabber.summary.bestVideo')
            : t('grabber.summary.video', { height: media.quality }),
      )
    }
    return parts.join(' · ')
  })
</script>

<!-- Open, the options share the height with the links: each part scrolls on its own. -->
<section class="border-t border-line {open ? 'sm:flex sm:min-h-0 sm:flex-1 sm:flex-col' : ''}">
  <button
    type="button"
    aria-expanded={open}
    aria-controls={id}
    onclick={() => (open = !open)}
    class="flex w-full shrink-0 items-center gap-3 px-5 py-3 text-left transition-colors hover:bg-sunken/60"
  >
    <SlidersHorizontal size={15} class="shrink-0 text-fg-faint" aria-hidden="true" />
    <span class="shrink-0 font-medium">{t('grabber.options')}</span>
    <span class="min-w-0 flex-1 truncate text-small text-fg-muted first-letter:uppercase">{summary}</span>
    <ChevronDown
      size={16}
      class="shrink-0 text-fg-faint transition-transform duration-150 {open ? 'rotate-180' : ''}"
      aria-hidden="true"
    />
  </button>
  {#if open}
    <div
      {id}
      class="grid gap-x-8 gap-y-6 px-5 pt-1 pb-5 sm:min-h-0 sm:flex-1 sm:overflow-y-auto {hasMedia
        ? 'sm:grid-cols-2'
        : 'max-w-md'}"
    >
      <PackageOptionsFields bind:options />
      {#if hasMedia}
        <section aria-labelledby="{id}-media" class="space-y-3">
          <h3 id="{id}-media" class="heading font-semibold">{t('media.title')}</h3>
          <MediaOptionsFields bind:options={media} />
        </section>
      {/if}
    </div>
  {/if}
</section>
