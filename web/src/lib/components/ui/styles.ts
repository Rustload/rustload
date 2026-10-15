// Shared class recipes so buttons, links and inputs look identical wherever they're rendered.

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger'
export type ButtonSize = 'sm' | 'md'
/** `xs` fits inside a chip or a tag. */
export type IconButtonSize = 'xs' | ButtonSize

const buttonBase =
  'inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-control font-medium select-none ' +
  'transition-[background-color,border-color,color,filter] duration-100 disabled:pointer-events-none'

// A primary action that can't run yet turns neutral: the signal colour only shows when it can be used
// (or while it runs).
const buttonVariants: Record<ButtonVariant, string> = {
  primary:
    'bg-signal text-signal-fg hover:brightness-[1.06] active:brightness-95 ' +
    'disabled:not-aria-busy:bg-sunken disabled:not-aria-busy:text-fg-faint',
  secondary: 'border border-line bg-raised text-fg hover:border-line-strong hover:bg-sunken disabled:opacity-50',
  ghost: 'text-fg-muted hover:bg-sunken hover:text-fg disabled:opacity-50',
  danger: 'bg-bad text-bad-fg hover:brightness-110 active:brightness-95 disabled:opacity-50',
}

const buttonSizes: Record<ButtonSize, string> = {
  sm: 'h-7 px-2.5 text-small',
  md: 'h-9 px-3.5 text-sm',
}

const iconButtonSizes: Record<IconButtonSize, string> = {
  xs: 'size-4',
  sm: 'size-7',
  md: 'size-9',
}

export function buttonClass(variant: ButtonVariant = 'secondary', size: ButtonSize = 'md', extra = ''): string {
  return `${buttonBase} ${buttonVariants[variant]} ${buttonSizes[size]} ${extra}`
}

export function iconButtonClass(variant: ButtonVariant = 'ghost', size: IconButtonSize = 'md', extra = ''): string {
  return `${buttonBase} ${buttonVariants[variant]} ${iconButtonSizes[size]} ${extra}`
}

export const inputClass =
  'block w-full min-w-0 rounded-control border border-line bg-sunken px-3 h-9 text-sm text-fg ' +
  'placeholder:text-fg-faint transition-colors hover:border-line-strong focus:border-info focus:outline-none ' +
  'focus-visible:outline-none focus:ring-2 focus:ring-info/25 disabled:opacity-60 read-only:text-fg-muted'
