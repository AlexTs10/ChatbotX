// apps/builder/src/lib/locale.ts ("use server", cookie) for the browser.
import { defaultLocale, type Locale } from "@/i18n/config"

const COOKIE_NAME = "NEXT_LOCALE"
const COOKIE_RE = /(?:^|;\s*)NEXT_LOCALE=([^;]+)/

export function getUserLocale(): string {
  const match = document.cookie.match(COOKIE_RE)
  return match ? decodeURIComponent(match[1]) : defaultLocale
}

/** Sets the locale cookie; the page reloads so the new messages load. */
export function setUserLocale(locale: Locale): Promise<void> {
  // biome-ignore lint/suspicious/noDocumentCookie: the Cookie Store API is not in every browser
  document.cookie = `${COOKIE_NAME}=${encodeURIComponent(locale)}; path=/; max-age=31536000; samesite=lax`
  window.location.reload()
  return Promise.resolve()
}
