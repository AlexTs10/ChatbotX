// page: app/space/[workspaceId]/broadcasts/page.tsx
// page: app/space/[workspaceId]/broadcasts/create/page.tsx
// page: app/space/[workspaceId]/broadcasts/[broadcastId]/edit/page.tsx
// page: app/space/[workspaceId]/broadcasts/[broadcastId]/runs/page.tsx
// `broadcasts/layout.tsx`: `resolveGuardedWorkspaceId(params, "broadcast")`
// (404 without the permission). Membership is the workspace layout's check.
// The plan policy is the unrestricted one: Rust has no trial plan identity
// (`broadcast::plan_policy::resolve_for_workspace`).
import { UNRESTRICTED_BROADCAST_PLAN_POLICY } from "@chatbotx.io/database/partials"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet, useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { Suspense } from "react"
import { BroadcastsTable } from "@/features/broadcasts/broadcasts-table"
import { BroadcastRunsView } from "@/features/broadcasts/components/broadcast-runs-view"
import { BroadcastsCalendar } from "@/features/broadcasts/components/broadcasts-calendar"
import { BroadcastsListShell } from "@/features/broadcasts/components/broadcasts-list-shell"
import { CreateBroadcastForm } from "@/features/broadcasts/create-broadcast-form"
import { BROADCASTS_PANEL_COOKIE } from "@/features/broadcasts/lib/broadcast-status"
import {
  resolveDateParam,
  resolveEndDateParam,
} from "@/features/broadcasts/lib/calendar-grid"
import { buildEditBroadcastDefaultValues } from "@/features/broadcasts/lib/create-broadcast-defaults"
import { listBroadcasts } from "@/features/broadcasts/queries"
import { listBroadcastsForCalendar } from "@/features/broadcasts/queries/list-broadcasts-for-calendar"
import { parseCreateBroadcastPrefill } from "@/features/broadcasts/schema/create-broadcast-prefill"
import { getBroadcastsSearchParamsCache } from "@/features/broadcasts/schema/query"
import { ContactStoreProvider } from "@/features/contacts/provider/contact-store-context"
import { FlowTemplateStoreProvider } from "@/features/flows/react-flow/stores/flow-template-store-provider"
import { WhatsappFlowStoreProvider } from "@/features/flows/react-flow/stores/whatsapp-flow-store-provider"
import { listIntegrationOpenaiCompatible } from "@/features/integration-openai-compatible/queries"
import { IntegrationStoreProvider } from "@/features/integration-whatsapp/provider/integration-store-context"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import { useQueryPromise } from "@/lib/query/use-query-promise"
import { serverQuery } from "@/lib/server-action"
import { getBrowserTimezone } from "@/lib/timezone"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"

function useIds() {
  return useParams({ strict: false }) as {
    workspaceId: string
    broadcastId?: string
  }
}

function readCookie(name: string): string | undefined {
  return document.cookie
    .split("; ")
    .find((part) => part.startsWith(`${name}=`))
    ?.slice(name.length + 1)
}

export function BroadcastsLayout() {
  const { permissions } = useWorkspacePage()
  if (!hasWorkspacePermission(permissions, "broadcast")) {
    return <NotFoundPage />
  }
  return (
    <Suspense>
      <Outlet />
    </Suspense>
  )
}

export function BroadcastsPage() {
  const { workspaceId } = useIds()
  const searchParams = useSearchParams()
  const search = getBroadcastsSearchParamsCache.parse(
    Object.fromEntries(searchParams),
  )
  const panelOpen = readCookie(BROADCASTS_PANEL_COOKIE) !== "false"
  const filtered = Boolean(search.name || search.status || search.kind)

  return (
    <BroadcastsListShell defaultPanelOpen={panelOpen}>
      <Suspense>
        {search.view === "calendar" ? (
          <CalendarView search={search} workspaceId={workspaceId} />
        ) : (
          <TableView
            filtered={filtered}
            search={search}
            workspaceId={workspaceId}
          />
        )}
      </Suspense>
    </BroadcastsListShell>
  )
}

// `parse` is overloaded: `ReturnType` takes the Promise one.
type Search = Awaited<ReturnType<typeof getBroadcastsSearchParamsCache.parse>>

function TableView(props: {
  search: Search
  workspaceId: string
  filtered: boolean
}) {
  const { view, range, date, endDate, ...rest } = props.search
  const input = { ...rest, workspaceId: props.workspaceId }
  const promises = useQueryPromise(
    ["broadcasts", props.workspaceId, JSON.stringify(input)],
    () => Promise.all([listBroadcasts(input)]),
  )
  return <BroadcastsTable filtered={props.filtered} promises={promises} />
}

function CalendarView(props: { search: Search; workspaceId: string }) {
  const { search, workspaceId } = props
  const timezone = getBrowserTimezone()
  const calendarDate = resolveDateParam(search.date, timezone)
  const calendarEndDate = resolveEndDateParam(search.endDate, calendarDate)
  const input = {
    workspaceId,
    range: search.range,
    date: calendarDate,
    endDate: calendarEndDate,
    status: search.status,
    kind: search.kind,
    name: search.name,
    timezone,
  }
  const { data: broadcasts } = useSuspenseQuery({
    queryKey: ["spa", "broadcastsCalendar", JSON.stringify(input)],
    queryFn: () => listBroadcastsForCalendar(input),
  })
  return (
    <BroadcastsCalendar
      broadcasts={broadcasts}
      date={calendarDate}
      endDate={calendarEndDate}
      range={search.range}
    />
  )
}

/** The providers + form of the create and edit pages. */
function BroadcastForm(
  props: Omit<
    Parameters<typeof CreateBroadcastForm>[0],
    "canViewEmailAndPhone" | "planPolicy" | "workspaceId"
  >,
) {
  const { workspaceId } = useIds()
  const { permissions } = useWorkspacePage()
  const { data: openaiCompatibleIntegrations } = useSuspenseQuery({
    queryKey: ["spa", "openaiCompatibleIntegrations", workspaceId],
    queryFn: () => listIntegrationOpenaiCompatible({ workspaceId }),
  })
  return (
    <IntegrationStoreProvider workspaceId={workspaceId}>
      <FlowTemplateStoreProvider
        includeAllTemplateStatuses
        openaiCompatibleIntegrations={openaiCompatibleIntegrations}
        workspaceId={workspaceId}
      >
        <WhatsappFlowStoreProvider workspaceId={workspaceId}>
          <ContactStoreProvider
            autoInitialize={false}
            workspaceId={workspaceId}
          >
            <CreateBroadcastForm
              {...props}
              canViewEmailAndPhone={hasWorkspacePermission(
                permissions,
                "emailAndPhone",
              )}
              planPolicy={UNRESTRICTED_BROADCAST_PLAN_POLICY}
              workspaceId={workspaceId}
            />
          </ContactStoreProvider>
        </WhatsappFlowStoreProvider>
      </FlowTemplateStoreProvider>
    </IntegrationStoreProvider>
  )
}

export function CreateBroadcastPage() {
  const { workspaceId } = useIds()
  const searchParams = useSearchParams()
  const prefill = parseCreateBroadcastPrefill(Object.fromEntries(searchParams))
  // The Ads deep link names a WhatsApp integration; a foreign or deleted id
  // preselects nothing.
  const { data: inboxId } = useSuspenseQuery({
    queryKey: [
      "spa",
      "broadcastPrefillInbox",
      workspaceId,
      prefill.integrationWhatsappId ?? "",
    ],
    queryFn: () =>
      prefill.integrationWhatsappId
        ? serverQuery<string | null>(
            "findIntegrationWhatsappInboxId",
            workspaceId,
            [
              workspaceId,
              { integrationWhatsappId: prefill.integrationWhatsappId },
            ],
          )
        : null,
  })
  return (
    <BroadcastForm
      initialChannel={prefill.channel}
      initialContactFilter={prefill.contactFilter}
      initialInboxIds={inboxId ? [inboxId] : undefined}
    />
  )
}

export function EditBroadcastPage() {
  const { workspaceId, broadcastId = "" } = useIds()
  // Only a `draft` is editable; anything else does not resolve.
  const { data: draft } = useSuspenseQuery({
    queryKey: ["spa", "broadcastDraft", workspaceId, broadcastId],
    queryFn: () =>
      serverQuery<Parameters<typeof buildEditBroadcastDefaultValues>[0] | null>(
        "findDraftBroadcast",
        workspaceId,
        [workspaceId, { broadcastId }],
      ),
  })
  const editDraft = draft ? buildEditBroadcastDefaultValues(draft) : null
  if (!editDraft) {
    return <NotFoundPage />
  }
  return (
    <BroadcastForm editDraft={editDraft} initialChannel={editDraft.channel} />
  )
}

export function BroadcastRunsPage() {
  const { workspaceId, broadcastId = "" } = useIds()
  // Only a repeating broadcast has runs.
  const { data: broadcast } = useSuspenseQuery({
    queryKey: ["spa", "broadcastRuns", workspaceId, broadcastId],
    queryFn: () =>
      serverQuery<Parameters<typeof BroadcastRunsView>[0]["broadcast"] | null>(
        "findRecurringBroadcast",
        workspaceId,
        [workspaceId, { broadcastId }],
      ),
  })
  if (!broadcast) {
    return <NotFoundPage />
  }
  return <BroadcastRunsView broadcast={broadcast} workspaceId={workspaceId} />
}
