<script lang="ts">
  import type { HTMLTextareaAttributes } from 'svelte/elements'
  import { inputClass } from './styles'

  type Props = Omit<HTMLTextareaAttributes, 'value'> & {
    value: string
    mono?: boolean
    element?: HTMLTextAreaElement
  }

  let { value = $bindable(), element = $bindable(), mono = false, class: className = '', ...rest }: Props = $props()
</script>

<textarea
  bind:this={element}
  bind:value
  class="{inputClass} h-auto resize-y py-2 leading-relaxed {mono ? 'font-mono text-small' : ''} {className}"
  {...rest}></textarea>
