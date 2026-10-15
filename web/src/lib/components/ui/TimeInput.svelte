<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements'
  import { inputClass } from './styles'

  /** A time of day as "HH:MM". */
  type Props = Omit<HTMLInputAttributes, 'type' | 'value'> & { value: string }

  let { value = $bindable(), class: className = '', ...rest }: Props = $props()
</script>

<input type="time" bind:value class="{inputClass} num {className}" {...rest} />
