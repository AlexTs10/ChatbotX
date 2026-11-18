// next-intl's request config (apps/builder/src/i18n/request.ts) for the SPA:
// the locale comes from the NEXT_LOCALE cookie, the zone from the browser,
// and each locale's messages are a lazy chunk.

import { type ReactNode, use } from "react"
import type { AbstractIntlMessages } from "use-intl"
import { IntlProvider } from "use-intl"
import { getUserLocale } from "@/lib/locale"
import { getBrowserTimezone } from "@/lib/timezone"
import { resolveLocale } from "./config"

// ponytail: messages stay in apps/builder/messages until the builder is removed.
const loaders = import.meta.glob<AbstractIntlMessages>(
  "../../../builder/messages/*.json",
  {
    import: "default",
  },
)

const load = (locale: string) =>
  loaders[`../../../builder/messages/${locale}.json`]?.() ?? Promise.resolve({})

export const locale = resolveLocale(getUserLocale())
const messagesPromise = Promise.all([
  load(locale),
  locale === "en" ? null : load("en"),
])

function lookup(messages: AbstractIntlMessages | null, path: string): unknown {
  return path
    .split(".")
    .reduce<unknown>(
      (value, segment) =>
        typeof value === "object" && value !== null
          ? (value as Record<string, unknown>)[segment]
          : undefined,
      messages,
    )
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [messages, english] = use(messagesPromise)
  return (
    <IntlProvider
      getMessageFallback={({ key, namespace }) => {
        const path = namespace ? `${namespace}.${key}` : key
        const fallback = lookup(english ?? messages, path)
        return typeof fallback === "string" ? fallback : path
      }}
      locale={locale}
      messages={messages}
      onError={() => undefined}
      timeZone={getBrowserTimezone()}
    >
      {children}
    </IntlProvider>
  )
}

const loadPair = (target: string) =>
  Promise.all([load(target), target === "en" ? null : load("en")])

const localeMessages = new Map<string, ReturnType<typeof loadPair>>()

/**
 * A subtree in another locale than the viewer's: the public web shop speaks
 * the workspace language, whoever opens the link (`loadWorkspaceMessages`).
 */
export function LocaleProvider({
  locale: target,
  children,
}: {
  locale: string
  children: ReactNode
}) {
  const resolved = resolveLocale(target)
  let pending = localeMessages.get(resolved)
  if (!pending) {
    pending = loadPair(resolved)
    localeMessages.set(resolved, pending)
  }
  const [messages, english] = use(pending)
  return (
    <IntlProvider
      getMessageFallback={({ key, namespace }) => {
        const path = namespace ? `${namespace}.${key}` : key
        const fallback = lookup(english ?? messages, path)
        return typeof fallback === "string" ? fallback : path
      }}
      locale={resolved}
      messages={messages}
      onError={() => undefined}
      timeZone={getBrowserTimezone()}
    >
      {children}
    </IntlProvider>
  )
}
