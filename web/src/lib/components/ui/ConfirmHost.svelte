<script lang="ts">
  import { t } from '$lib/i18n/index.svelte'
  import { ui } from '$lib/stores/ui.svelte'
  import Button from './Button.svelte'
  import Checkbox from './Checkbox.svelte'
  import Dialog from './Dialog.svelte'

  // Single dialog instance backing every `ui.confirm()` call.
  let open = $state(false)
  let checked = $state(false)
  let answered = false

  const request = $derived(ui.confirmRequest)

  $effect(() => {
    if (request) {
      checked = false
      answered = false
      open = true
    }
  })

  function answer(confirmed: boolean) {
    answered = true
    open = false
    ui.answerConfirm({ confirmed, checked })
  }

  function onclose() {
    if (!answered) ui.answerConfirm({ confirmed: false, checked: false })
  }
</script>

{#if request}
  <Dialog bind:open title={request.title} size="sm" {onclose} initialFocus="[data-confirm]">
    {#if request.body}
      <p class="text-fg-muted">{request.body}</p>
    {/if}
    {#if request.checkbox}
      <Checkbox bind:checked label={request.checkbox} class="mt-4" />
    {/if}
    {#snippet footer()}
      <Button onclick={() => answer(false)}>{t('common.cancel')}</Button>
      <Button variant={request.danger ? 'danger' : 'primary'} data-confirm onclick={() => answer(true)}>
        {request.confirmLabel}
      </Button>
    {/snippet}
  </Dialog>
{/if}
