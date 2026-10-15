<script lang="ts">
  import ChevronRight from '@lucide/svelte/icons/chevron-right'
  import { t } from '$lib/i18n/index.svelte'
  import IconButton from './IconButton.svelte'

  /** Shows or hides the list of files under a row; the chevron turns down while it's open. */
  type Props = {
    expanded: boolean
    /** Id of the list it shows. */
    controls?: string
    class?: string
  }

  let { expanded = $bindable(), controls, class: className = '' }: Props = $props()
</script>

<IconButton
  icon={ChevronRight}
  label={expanded ? t('common.hideFiles') : t('common.showFiles')}
  size="sm"
  class="shrink-0 {className}"
  iconClass="transition-transform duration-150 {expanded ? 'rotate-90' : ''}"
  aria-expanded={expanded}
  aria-controls={controls}
  onclick={() => (expanded = !expanded)}
/>
