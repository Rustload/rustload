<script lang="ts">
  /** The line a `flush` Panel shows instead of an empty list. */
  let { text }: { text: string } = $props()
</script>

<p class="border-t border-line px-5 py-6 text-fg-muted">{text}</p>
