import { t } from '$lib/i18n/index.svelte'

/** The server's rule (`MIN_PASSWORD_LEN`), counted in characters. */
export const MIN_PASSWORD_LENGTH = 8

export type PasswordCheck = { next: string | null; confirm: string | null; ok: boolean }

/** Field errors for a new password and its confirmation (empty fields aren't flagged yet); `ok` once both pass. */
export function passwordErrors(next: string, confirm: string): PasswordCheck {
  const length = [...next].length
  const tooShort = length > 0 && length < MIN_PASSWORD_LENGTH
  return {
    next: tooShort ? t('errors.passwordTooShort', { min: MIN_PASSWORD_LENGTH }) : null,
    confirm: confirm.length > 0 && confirm !== next ? t('errors.passwordMismatch') : null,
    ok: length >= MIN_PASSWORD_LENGTH && confirm === next,
  }
}
