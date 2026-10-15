<script lang="ts">
  import { untrack } from 'svelte'
  import TextArea from './TextArea.svelte'

  /** Edits a string list as one entry per line. */
  type Props = {
    values: string[]
    id?: string
    describedBy?: string
    rows?: number
    placeholder?: string
  }

  let { values = $bindable(), id, describedBy, rows = 4, placeholder }: Props = $props()

  let text = $state('')

  function parse(value: string): string[] {
    return value
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean)
  }

  // Re-sync the text when the list changes from outside (e.g. "discard changes").
  $effect.pre(() => {
    const next = values
    untrack(() => {
      if (JSON.stringify(parse(text)) !== JSON.stringify(next)) text = next.join('\n')
    })
  })
</script>

<TextArea
  {id}
  {rows}
  {placeholder}
  mono
  spellcheck={false}
  autocomplete="off"
  aria-describedby={describedBy}
  bind:value={text}
  oninput={() => (values = parse(text))}
/>
