// The browser zone. The server reads it from the NEXT_TIMEZONE cookie
// (apps/builder/src/lib/timezone.ts); `main.tsx` writes the cookie.
import { resolveFilterTimezone } from "@chatbotx.io/utils/datetime"
import {
  TIMEZONE_COOKIE_MAX_AGE_SECONDS,
  TIMEZONE_COOKIE_NAME,
} from "./timezone-cookie"

export function getBrowserTimezone(): string {
  return resolveFilterTimezone(Intl.DateTimeFormat().resolvedOptions().timeZone)
}

export function syncTimezoneCookie() {
  // biome-ignore lint/suspicious/noDocumentCookie: the Cookie Store API is not in every browser
  document.cookie = `${TIMEZONE_COOKIE_NAME}=${encodeURIComponent(getBrowserTimezone())}; path=/; max-age=${TIMEZONE_COOKIE_MAX_AGE_SECONDS}; samesite=lax`
}
