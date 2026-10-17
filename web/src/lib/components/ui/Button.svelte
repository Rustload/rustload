<script lang="ts">
  import type { LucideIcon } from '@lucide/svelte'
  import type { Snippet } from 'svelte'
  import type { HTMLButtonAttributes } from 'svelte/elements'
  import Spinner from './Spinner.svelte'
  import { buttonClass, type ButtonSize, type ButtonVariant } from './styles'

  type Props = HTMLButtonAttributes & {
    variant?: ButtonVariant
    size?: ButtonSize
    icon?: LucideIcon
    loading?: boolean
    /** Renders a link styled as a button. */
    href?: string
    external?: boolean
    /** For a link: saves its target as a file instead of opening it. */
    download?: boolean
    children?: Snippet
  }

  let {
    variant = 'secondary',
    size = 'md',
    icon: Icon,
    loading = false,
    href,
    external = false,
    download = false,
    type = 'button',
    disabled,
    class: className = '',
    children,
    ...rest
  }: Props = $props()

  const iconSize = $derived(size === 'sm' ? 14 : 16)
</script>

{#snippet content()}
  {#if loading}
    <Spinner size={iconSize} />
  {:else if Icon}
    <Icon size={iconSize} strokeWidth={2} aria-hidden="true" />
  {/if}
  {@render children?.()}
{/snippet}

{#if href}
  <a
    {href}
    class={buttonClass(variant, size, String(className))}
    target={external ? '_blank' : undefined}
    rel={external ? 'noopener noreferrer' : undefined}
    download={download || undefined}
  >
    {@render content()}
  </a>
{:else}
  <button
    {type}
    disabled={disabled || loading}
    aria-busy={loading || undefined}
    class={buttonClass(variant, size, String(className))}
    {...rest}
  >
    {@render content()}
  </button>
{/if}
