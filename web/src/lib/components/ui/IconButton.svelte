<script lang="ts">
  import type { LucideIcon } from '@lucide/svelte'
  import type { HTMLButtonAttributes } from 'svelte/elements'
  import { iconButtonClass, type ButtonVariant, type IconButtonSize } from './styles'

  type Props = HTMLButtonAttributes & {
    icon: LucideIcon
    /** Accessible name, also shown as a tooltip. */
    label: string
    variant?: ButtonVariant
    size?: IconButtonSize
    /** Classes for the icon itself (e.g. a rotation). */
    iconClass?: string
  }

  let {
    icon: Icon,
    label,
    variant = 'ghost',
    size = 'md',
    iconClass,
    type = 'button',
    class: className = '',
    ...rest
  }: Props = $props()

  const ICON_SIZE: Record<IconButtonSize, number> = { xs: 12, sm: 15, md: 17 }
</script>

<button {type} aria-label={label} title={label} class={iconButtonClass(variant, size, String(className))} {...rest}>
  <Icon size={ICON_SIZE[size]} strokeWidth={2} class={iconClass} aria-hidden="true" />
</button>
