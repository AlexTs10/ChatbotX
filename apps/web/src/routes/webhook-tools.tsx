// page: app/space/[workspaceId]/inbound-webhooks/page.tsx
// page: app/space/[workspaceId]/inbound-webhooks/[id]/page.tsx
// page: app/space/[workspaceId]/inbound-webhooks/[id]/analytics/page.tsx
// page: app/space/[workspaceId]/(has-folder)/webhooks/layout.tsx
// page: app/space/[workspaceId]/(has-folder)/webhooks/page.tsx
// page: app/space/[workspaceId]/webhooks/[id]/edit/page.tsx
// page: app/space/[workspaceId]/magic-links/page.tsx
// page: app/space/[workspaceId]/magic-links/[magicLinkId]/analytics/page.tsx
// page: app/space/[workspaceId]/(integrations)/google-sheets/page.tsx
// The tool screens of Q.D13a: inbound webhooks, outbound webhooks, magic
// links and Google Sheets. The webhook pages are gated with
// `requireWorkspacePermission(…, "superAdmin")`; the queries check it again.
import { folderTypes } from "@chatbotx.io/database/partials"
import { ORPCError } from "@orpc/client"
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@chatbotx.io/ui/components/ui/card"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet, useParams } from "@tanstack/react-router"
import { ArrowLeftIcon } from "lucide-react"
import Link from "next/link"
import { useSearchParams } from "next/navigation"
import { useFormatter, useTranslations } from "next-intl"
import { type ReactNode, Suspense } from "react"
import { MagicLinkAnalyticsClient } from "@/app/space/[workspaceId]/magic-links/[magicLinkId]/analytics/magic-link-analytics-client"
import { AppBreadcrumb } from "@/components/app-breadcrumb"
import { FolderStoreProvider } from "@/features/folders/provider/folder-store-context"
import SharedFolderSlot from "@/features/folders/shared-folder-slot"
import { InboundWebhookEditor } from "@/features/inbound-webhooks/components/inbound-webhook-editor"
import { InboundWebhookRequestsPanel } from "@/features/inbound-webhooks/components/inbound-webhook-requests-panel"
import { InboundWebhooksTable } from "@/features/inbound-webhooks/components/inbound-webhooks-table"
import {
  findInboundWebhookForEditor,
  listInboundWebhookRequests,
  listInboundWebhooks,
} from "@/features/inbound-webhooks/queries"
import {
  inboundWebhookRequestsSearchParamsCache,
  listInboundWebhooksSearchParamsCache,
  REQUESTS_PER_PAGE,
} from "@/features/inbound-webhooks/schema/query"
import { MagicLinksTable } from "@/features/magic-links/magic-links-table"
import { findMagicLink, listMagicLinks } from "@/features/magic-links/queries"
import { listMagicLinksSearchParamsCache } from "@/features/magic-links/schema/query"
import { listSpreadsheets } from "@/features/spreadsheets/queries/list-spreadsheet.queries"
import { SpreadsheetsTable } from "@/features/spreadsheets/spreadsheets-table"
import {
  findWebhook,
  getWebhookStoreSource,
  getWebhooks,
} from "@/features/webhooks/queries"
import { getWebhooksSearchParamsCache } from "@/features/webhooks/schema/get-webhook-schema"
import UpdateWebhookForm from "@/features/webhooks/update-webhook-form"
import { WebhooksTable } from "@/features/webhooks/webhooks-table"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import { useQueryPromise } from "@/lib/query/use-query-promise"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"

function useIds() {
  return useParams({ strict: false }) as { workspaceId: string; id?: string }
}

/** `requireWorkspacePermission(workspaceId, "superAdmin")`. */
function SuperAdminOnly(props: { children: ReactNode }) {
  const { permissions } = useWorkspacePage()
  if (!hasWorkspacePermission(permissions, "superAdmin")) {
    return <NotFoundPage />
  }
  return <Suspense>{props.children}</Suspense>
}

/** A detail read: `null` when the row is not in the workspace (`notFound()`). */
async function orNull<T>(load: Promise<T>): Promise<T | null> {
  try {
    return await load
  } catch (error) {
    if (error instanceof ORPCError && error.status === 404) {
      return null
    }
    throw error
  }
}

function useSearch() {
  const searchParams = useSearchParams()
  return {
    key: searchParams.toString(),
    values: Object.fromEntries(searchParams) as Record<string, string>,
  }
}

function InboundWebhooksBreadcrumb(props: {
  workspaceId: string
  items?: { label: string; href: string }[]
}) {
  const t = useTranslations()
  return (
    <AppBreadcrumb
      items={[
        { label: t("tools.title"), href: `/space/${props.workspaceId}/tools` },
        {
          label: t("inboundWebhooks.title"),
          href: props.items ? `/space/${props.workspaceId}/inbound-webhooks` : "",
        },
        ...(props.items ?? []),
      ]}
    />
  )
}

export function InboundWebhooksPage() {
  return (
    <SuperAdminOnly>
      <InboundWebhooksList />
    </SuperAdminOnly>
  )
}

function InboundWebhooksList() {
  const { workspaceId } = useIds()
  const search = useSearch()
  const parsed = listInboundWebhooksSearchParamsCache.parse(search.values)
  const promises = useQueryPromise(
    ["listInboundWebhooks", workspaceId, search.key],
    () => Promise.all([listInboundWebhooks({ ...parsed, workspaceId })]),
  )
  return (
    <div className="flex flex-col gap-4">
      <InboundWebhooksBreadcrumb workspaceId={workspaceId} />
      <InboundWebhooksTable promises={promises} workspaceId={workspaceId} />
    </div>
  )
}

export function InboundWebhookEditorPage() {
  return (
    <SuperAdminOnly>
      <InboundWebhookEditorView />
    </SuperAdminOnly>
  )
}

function InboundWebhookEditorView() {
  const { workspaceId, id = "" } = useIds()
  const webhook = useSuspenseQuery({
    queryKey: ["spa", "findInboundWebhookForEditor", workspaceId, id],
    queryFn: () => orNull(findInboundWebhookForEditor({ workspaceId, id })),
  }).data
  if (!webhook) {
    return <NotFoundPage />
  }
  return (
    <div className="flex flex-col gap-4 pb-20">
      <InboundWebhooksBreadcrumb
        items={[{ label: webhook.name, href: "" }]}
        workspaceId={workspaceId}
      />
      <InboundWebhookEditor webhook={webhook} workspaceId={workspaceId} />
    </div>
  )
}

export function InboundWebhookAnalyticsPage() {
  return (
    <SuperAdminOnly>
      <InboundWebhookAnalyticsView />
    </SuperAdminOnly>
  )
}

function InboundWebhookAnalyticsView() {
  const t = useTranslations()
  const format = useFormatter()
  const { workspaceId, id = "" } = useIds()
  const search = useSearch()
  const { requestsPage } = inboundWebhookRequestsSearchParamsCache.parse(
    search.values,
  )
  const loaded = useSuspenseQuery({
    queryKey: ["spa", "inboundWebhookAnalytics", workspaceId, id, requestsPage],
    queryFn: () =>
      orNull(
        Promise.all([
          findInboundWebhookForEditor({ workspaceId, id }),
          listInboundWebhookRequests({
            workspaceId,
            inboundWebhookId: id,
            page: requestsPage,
            perPage: REQUESTS_PER_PAGE,
          }),
        ]),
      ),
  }).data
  if (!loaded) {
    return <NotFoundPage />
  }
  const [webhook, requests] = loaded
  const editorHref = `/space/${workspaceId}/inbound-webhooks/${id}`
  return (
    <div className="flex flex-col gap-4">
      <InboundWebhooksBreadcrumb
        items={[
          { label: webhook.name, href: editorHref },
          { label: t("inboundWebhooks.analytics.title"), href: "" },
        ]}
        workspaceId={workspaceId}
      />
      <Card>
        <CardHeader>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <CardTitle className="font-bold text-xl">
                {webhook.name}
              </CardTitle>
            </div>
            <Link
              className="inline-flex items-center gap-1 text-sm hover:underline"
              href={editorHref}
            >
              <ArrowLeftIcon aria-hidden className="size-4 rtl:rotate-180" />
              {t("inboundWebhooks.analytics.back")}
            </Link>
          </div>
        </CardHeader>
        <CardContent>
          <p className="text-muted-foreground text-sm">
            {t("inboundWebhooks.analytics.totalRequests")}
          </p>
          <p className="font-bold text-3xl">
            {format.number(webhook.requestCount)}
          </p>
          <p className="text-muted-foreground text-xs">
            {t("inboundWebhooks.analytics.totalRequestsHint")}
          </p>
        </CardContent>
      </Card>
      <InboundWebhookRequestsPanel
        inboundWebhookId={id}
        page={requestsPage}
        pageCount={requests.pageCount}
        requests={requests.data}
        workspaceId={workspaceId}
      />
    </div>
  )
}

// ---- Outbound webhooks ------------------------------------------------------

export function WebhooksLayout() {
  const { workspaceId } = useIds()
  return (
    <SuperAdminOnly>
      <FolderStoreProvider
        autoInitialize={true}
        folderType={folderTypes.enum.webhook}
        workspaceId={workspaceId}
      >
        <Suspense>
          <Outlet />
        </Suspense>
      </FolderStoreProvider>
    </SuperAdminOnly>
  )
}

export function WebhooksPage() {
  const { workspaceId } = useIds()
  const t = useTranslations()
  const search = useSearch()
  const parsed = getWebhooksSearchParamsCache.parse(search.values)
  const promises = useQueryPromise(
    ["getWebhooks", workspaceId, search.key],
    () => Promise.all([getWebhooks({ ...parsed, workspaceId })]),
  )
  return (
    <Card>
      <CardHeader>
        <CardTitle className="font-bold text-xl">{t("webhooks.title")}</CardTitle>
      </CardHeader>
      <CardContent>
        <SharedFolderSlot
          folderType={folderTypes.enum.webhook}
          workspaceId={workspaceId}
        />
        <Suspense>
          <WebhooksTable
            folderId={parsed.folderId}
            promises={promises}
            workspaceId={workspaceId}
          />
        </Suspense>
      </CardContent>
    </Card>
  )
}

export function EditWebhookPage() {
  return (
    <SuperAdminOnly>
      <EditWebhookView />
    </SuperAdminOnly>
  )
}

function EditWebhookView() {
  const { workspaceId, id = "" } = useIds()
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "findWebhook", workspaceId, id],
    queryFn: () =>
      Promise.all([
        orNull(findWebhook({ workspaceId, id })),
        getWebhookStoreSource(workspaceId),
      ]),
  })
  const [webhook, storeSource] = data
  if (!webhook) {
    return <NotFoundPage />
  }
  return (
    <UpdateWebhookForm
      storeSource={storeSource}
      webhook={webhook}
      workspaceId={workspaceId}
    />
  )
}

// ---- Magic links ------------------------------------------------------------

export function MagicLinksPage() {
  const { workspaceId } = useIds()
  const t = useTranslations()
  const search = useSearch()
  const parsed = listMagicLinksSearchParamsCache.parse(search.values)
  const promises = useQueryPromise(
    ["listMagicLinks", workspaceId, search.key],
    () => Promise.all([listMagicLinks({ ...parsed, workspaceId })]),
  )
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb
        items={[
          { label: t("tools.title"), href: `/space/${workspaceId}/tools` },
          { label: t("magicLinks.title"), href: "" },
        ]}
      />
      <Suspense>
        <MagicLinksTable promises={promises} workspaceId={workspaceId} />
      </Suspense>
    </div>
  )
}

export function MagicLinkAnalyticsPage() {
  const { workspaceId, magicLinkId = "" } = useParams({
    strict: false,
  }) as { workspaceId: string; magicLinkId?: string }
  const { data: magicLink } = useSuspenseQuery({
    queryKey: ["spa", "findMagicLink", workspaceId, magicLinkId],
    queryFn: () => findMagicLink({ workspaceId, id: magicLinkId }),
  })
  if (!magicLink) {
    return <NotFoundPage />
  }
  return (
    <div className="container mx-auto flex flex-col gap-6 py-6">
      <MagicLinkAnalyticsClient
        linkId={magicLinkId}
        linkName={magicLink.name}
        workspaceId={workspaceId}
      />
    </div>
  )
}

// ---- Google Sheets ----------------------------------------------------------

export function GoogleSheetsPage() {
  const { workspaceId } = useIds()
  const t = useTranslations()
  const search = useSearch()
  const page = Number(search.values.page)
  const perPage = Number(search.values.perPage)
  const promises = useQueryPromise(
    ["listSpreadsheets", workspaceId, search.key],
    () =>
      Promise.all([
        listSpreadsheets({
          workspaceId,
          name: search.values.name,
          page: Number.isFinite(page) ? page : undefined,
          perPage: Number.isFinite(perPage) ? perPage : undefined,
        }),
      ]),
  )
  return (
    <div className="space-y-4">
      <h3 className="font-bold text-xl">{t("googleSheets.title")}</h3>
      <Suspense>
        <SpreadsheetsTable promises={promises} workspaceId={workspaceId} />
      </Suspense>
    </div>
  )
}
