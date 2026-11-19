// page: app/unsubscribe/page.tsx
// page: app/oauth/consent/page.tsx
// page: app/connect/[sessionId]/page.tsx
// page: app/l/[workspaceId]/[id]/page.tsx
// page: app/l/[workspaceId]/p/[id]/page.tsx
// page: app/bs/[workspaceId]/[webchatId]/page.tsx
// page: app/extensions/datetime-picker/page.tsx
// page: app/(no-sidebar)/webchat/page.tsx
// page: app/go/[workspaceId]/page.tsx (server-rendered: crates/api/src/spa/public_pages.rs)
//
// The public pages (no builder login, except the OAuth consent). Their
// loaders run in the Rust server (crates/api/src/spa/public_pages.rs);
// `/go/[workspaceId]` is server-rendered there (App Links meta tags).
import { ORPCError } from "@orpc/client"
import { useQuery, useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import Script from "next/script"
import { useTranslations } from "next-intl"
import { type ReactNode, useEffect } from "react"
import { SimulatorWidget } from "@/features/bot-simulator/components/simulator-widget"
import { DateTimePickerForm } from "@/features/get-user-data-webview/components/date-time-picker-form"
import { InboxListLandingPage } from "@/features/inboxes/components/landing-inbox-list"
import { CustomWidgetStyle } from "@/features/integration-webchat/lib/widget-css"
import { GuestSessionStoreProvider } from "@/features/integration-webchat/providers/store/guest-session-provider"
import type { WebchatClientConfig } from "@/features/integration-webchat/providers/store/lib/webchat-client-config"
import { WebchatWrapper } from "@/features/integration-webchat/webchat-wrapper"
import { OAuthConsentForm } from "@/features/oauth-consent/consent-form"
import { LocaleProvider } from "@/i18n/provider"
import { client } from "@/lib/orpc/orpc"
import { loadPage } from "@/lib/server-action"
import { NotFoundPage } from "@/routes/root"

const search = () => new URLSearchParams(window.location.search)

/** A loader read; `null` when the server answers 404 (`notFound()`). */
function usePublicPage<T>(name: string, input: Record<string, unknown>) {
  return useSuspenseQuery({
    queryKey: ["spa", "public", name, input],
    queryFn: async () => {
      try {
        return await loadPage<T>(name, input)
      } catch (error) {
        if (error instanceof ORPCError && error.status === 404) {
          return null
        }
        throw error
      }
    },
  }).data
}

function Message(props: {
  title?: string
  description: string
  children?: ReactNode
}) {
  return (
    <div className="flex h-screen w-screen items-center justify-center">
      <div className="max-w-sm text-center">
        {props.title && <h1 className="font-semibold text-xl">{props.title}</h1>}
        <p className="mt-2 text-muted-foreground">{props.description}</p>
        {props.children}
      </div>
    </div>
  )
}

export function UnsubscribePage() {
  const t = useTranslations("unsubscribePage")
  const page = usePublicPage<{ state: "invalid" | "unavailable" | "done" }>(
    "unsubscribe",
    { token: search().get("token") ?? "" },
  )
  const state = page?.state ?? "invalid"
  if (state === "done") {
    return <Message description={t("description")} title={t("title")} />
  }
  return (
    <Message
      description={t(`${state}Description`)}
      title={t(`${state}Title`)}
    />
  )
}

type ConsentPage = {
  clientName: string
  oauthQuery: string
  scopes: string[]
  userEmail: string
  workspaces: { id: string; name: string }[]
}

export function OAuthConsentPage() {
  const oauthQuery = window.location.search.replace(/^\?/, "")
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "oauthConsentPage", oauthQuery],
    queryFn: async (): Promise<ConsentPage | null> => {
      try {
        return await client.authActions.oauthConsentPage({ oauthQuery })
      } catch (error) {
        if (error instanceof ORPCError && error.status === 404) {
          return null
        }
        throw error
      }
    },
  })
  return data ? <OAuthConsentForm {...data} /> : <NotFoundPage />
}

type ConnectKind =
  | "invalid"
  | "processing"
  | "completed"
  | "cancelled"
  | "expired"
  | "failed"

/** Re-reads every 3 s while the session is still in flight. */
export function ConnectSessionPage() {
  const t = useTranslations("connectSessionPage")
  const { sessionId } = useParams({ strict: false }) as { sessionId: string }
  const { data } = useQuery({
    queryKey: ["spa", "public", "connectSession", sessionId],
    queryFn: () =>
      loadPage<{ kind: ConnectKind }>("connectSession", { sessionId }),
    refetchInterval: (query) =>
      query.state.data?.kind === "processing" ? 3000 : false,
  })
  if (!data) {
    return null
  }
  return (
    <Message
      description={t(`${data.kind}Description`)}
      title={t(`${data.kind}Title`)}
    />
  )
}

type InboxLink = Parameters<typeof InboxListLandingPage>[0]["inboxLinks"][number]

function Landing(props: { poll: boolean }) {
  const { workspaceId, id } = useParams({ strict: false }) as {
    workspaceId: string
    id: string
  }
  const page = usePublicPage<{ links: InboxLink[] }>("landing", {
    workspaceId,
    id,
    poll: props.poll,
  })
  const single = page?.links.length === 1 ? page.links[0].url : null
  useEffect(() => {
    if (single) {
      window.location.replace(single)
    }
  }, [single])
  if (!page) {
    return <NotFoundPage />
  }
  return single ? null : <InboxListLandingPage inboxLinks={page.links} />
}

export const LandingPage = () => <Landing poll={false} />
export const PollLandingPage = () => <Landing poll />

export function BotSimulatorPage() {
  const t = useTranslations()
  const { workspaceId, webchatId } = useParams({ strict: false }) as {
    workspaceId: string
    webchatId: string
  }
  const page = usePublicPage<{
    brandColor: string
    webchatId: string
    websiteUrl: string
  }>("botSimulator", { workspaceId, webchatId, url: search().get("url") })
  useEffect(() => {
    document.title = t("botSimulator.title")
  }, [t])
  if (!page) {
    return <NotFoundPage />
  }
  return (
    <main className="fixed inset-0 bg-background">
      {/* Sandboxed without allow-top-navigation so a frame-busting site
          cannot navigate the simulator away from itself. */}
      <iframe
        className="size-full border-0"
        sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox"
        src={page.websiteUrl}
        title={t("botSimulator.websitePreview")}
      />
      <SimulatorWidget
        brandColor={page.brandColor}
        webchatId={page.webchatId}
        websiteUrl={page.websiteUrl}
        workspaceId={workspaceId}
      />
    </main>
  )
}

type DateTimePicker =
  | { state: "invalid" }
  | {
      state: "ok"
      mode: Parameters<typeof DateTimePickerForm>[0]["mode"]
      language: string
    }

export function DateTimePickerPage() {
  const t = useTranslations("userDataWebview")
  const token = search().get("token") ?? ""
  const page = usePublicPage<DateTimePicker>("datetimePicker", { token })
  if (!page) {
    return <NotFoundPage />
  }
  if (page.state === "invalid") {
    return (
      <div className="flex min-h-screen items-center justify-center p-6 text-center">
        <div className="max-w-md space-y-2">
          <h1 className="font-semibold text-2xl">
            {t("errors.invalidTokenTitle")}
          </h1>
          <p className="text-muted-foreground text-sm">
            {t("errors.invalidTokenDescription")}
          </p>
        </div>
      </div>
    )
  }
  return (
    <LocaleProvider locale={page.language}>
      <Script
        src="https://connect.facebook.net/en_US/messenger.Extensions.js"
        strategy="afterInteractive"
      />
      <DateTimePickerForm mode={page.mode} token={token} />
    </LocaleProvider>
  )
}

type WebchatPage =
  | { state: "unavailable" | "unauthorized" }
  | {
      state: "ok"
      accessToken: string
      guestConversationId: string
      workspaceLogoUrl: string | null
      customCss: string | null
      config: WebchatClientConfig
    }

export function WebchatPage() {
  const t = useTranslations("webchat")
  const params = search()
  const page = usePublicPage<WebchatPage>("webchat", {
    workspaceId: params.get("workspaceId"),
    webchatId: params.get("webchatId"),
    domain: params.get("domain"),
    parentOrigin: params.get("parentOrigin"),
    guestConversationId: params.get("guestConversationId"),
    // The builder read these from the document request (`Referer`,
    // `Sec-Fetch-Dest: iframe`).
    referrer: document.referrer,
    inIframe: window.self !== window.top,
  })
  if (!page) {
    return <NotFoundPage />
  }
  if (page.state !== "ok") {
    return (
      <main className="flex h-screen w-screen items-center justify-center bg-background p-6 text-center">
        <div className="max-w-sm space-y-2">
          {page.state === "unavailable" ? (
            <p className="text-muted-foreground text-sm">
              {t("chatUnavailable")}
            </p>
          ) : (
            <>
              <h1 className="font-semibold text-lg">
                {t("unauthorizedDomain.title")}
              </h1>
              <p className="text-muted-foreground text-sm">
                {t("unauthorizedDomain.description")}
              </p>
            </>
          )}
        </div>
      </main>
    )
  }
  return (
    <GuestSessionStoreProvider
      accessToken={page.accessToken}
      config={page.config}
      serverGuestConversationId={page.guestConversationId}
      workspaceLogoUrl={page.workspaceLogoUrl ?? undefined}
    >
      {page.customCss && <CustomWidgetStyle css={page.customCss} />}
      <WebchatWrapper referral={params.get("ref") ?? undefined} />
    </GuestSessionStoreProvider>
  )
}
