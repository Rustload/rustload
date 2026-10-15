<script lang="ts">
  import CircleAlert from '@lucide/svelte/icons/circle-alert'
  import CircleCheck from '@lucide/svelte/icons/circle-check'
  import Info from '@lucide/svelte/icons/info'
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert'
  import X from '@lucide/svelte/icons/x'
  import { t } from '$lib/i18n/index.svelte'
  import { toasts } from '$lib/stores/toasts.svelte'
  import { toastTone, toneText } from '$lib/utils/status'
  import IconButton from './IconButton.svelte'

  const icons = { success: CircleCheck, info: Info, warning: TriangleAlert, error: CircleAlert }
</script>

<div
  class="pointer-events-none fixed inset-x-3 bottom-20 z-50 flex flex-col items-end gap-2 sm:inset-x-auto sm:right-5 sm:bottom-5"
  aria-live="polite"
>
  {#each toasts.items as toast (toast.id)}
    {@const Icon = icons[toast.level]}
    <div
      role={toast.level === 'error' ? 'alert' : 'status'}
      class="toast pointer-events-auto flex w-full max-w-sm items-start gap-3 rounded-panel border border-line bg-raised
        py-2.5 pr-2 pl-3.5 shadow-pop"
    >
      <Icon size={17} class="mt-0.5 shrink-0 {toneText[toastTone[toast.level]]}" aria-hidden="true" />
      <p class="min-w-0 flex-1 py-px break-words">{toast.message}</p>
      <!-- Negative margins: the button is taller than a line of text but mustn't make the toast taller. -->
      <IconButton
        icon={X}
        label={t('common.close')}
        size="sm"
        class="-my-0.5 shrink-0"
        onclick={() => toasts.dismiss(toast.id)}
      />
    </div>
  {/each}
</div>

<style>
  .toast {
    animation: toast-in 180ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  @keyframes toast-in {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }
</style>
