// page: app/space/[workspaceId]/dashboard/layout.tsx
// page: app/space/[workspaceId]/dashboard/page.tsx
// page: app/space/[workspaceId]/dashboard/contacts/page.tsx
// page: app/space/[workspaceId]/dashboard/conversations/page.tsx
// page: app/space/[workspaceId]/dashboard/ads/page.tsx
// page: app/space/[workspaceId]/dashboard/ads/[channel]/page.tsx
// page: app/space/[workspaceId]/dashboard/ads/google/page.tsx
// page: app/space/[workspaceId]/dashboard/ads/conversion-events/page.tsx
// The layout's `analytics` permission check (404) and its inbox cards. The
// server data (`listInboxes`, `listAnalyticsFilterOptions`,
// `resolveAdsDashboardChannels`) comes from the one connected-inbox list
// (`inboxesAPI.listAllInboxesAuthenticatedAPI`), filtered as the builder
// queries filter it. Google Ads has no Rust domain: its nav entry stays off.
// `dashboard/ads/*` (ads domain, Q.D7) shows the not-available state.
import { ContactsDashboard } from "@chatbotx.io/analytics-nextjs/components/contacts-dashboard"
import { ConversationsDashboard } from "@chatbotx.io/analytics-nextjs/components/conversations-dashboard"
import {
  adsEligibleChannelTypes,
  channelTypes,
} from "@chatbotx.io/utils/channel"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet, useParams } from "@tanstack/react-router"
import { isCloud } from "@/env"
import { AnalyticsChannelFilter } from "@/features/analytics/components/analytics-channel-filter"
import { AnalyticsNav } from "@/features/analytics/components/analytics-nav"
import { InboxCardList } from "@/features/inboxes/components/inbox-card-list"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import { client } from "@/lib/orpc/orpc"
import { orpc } from "@/lib/orpc/query"
import { NotAvailablePage, NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"
import { Redirect } from "@/shims/redirect"

function useWorkspaceId() {
  return (useParams({ strict: false }) as { workspaceId: string }).workspaceId
}

/** The connected inboxes with their integrations, smtp excluded. */
function useDashboardInboxes() {
  const workspaceId = useWorkspaceId()
  const input = { workspaceId, includes: ["integration" as const] }
  const { data } = useSuspenseQuery({
    queryKey: orpc.inboxesAPI.listAllInboxesAuthenticatedAPI.queryKey({
      input,
    }),
    queryFn: () => client.inboxesAPI.listAllInboxesAuthenticatedAPI(input),
  })
  return data.data.filter((inbox) => inbox.channel !== "smtp")
}

export function DashboardLayout() {
  const workspaceId = useWorkspaceId()
  const { permissions, blocked, blockReason } = useWorkspacePage()
  if (!hasWorkspacePermission(permissions, "analytics")) {
    return <NotFoundPage />
  }

  return (
    <div className="flex flex-col gap-4">
      <DashboardInboxCards
        blocked={isCloud() && blocked}
        reason={isCloud() ? blockReason : null}
        superAdmin={hasWorkspacePermission(permissions, "superAdmin")}
        workspaceId={workspaceId}
      />
      <Outlet />
    </div>
  )
}

function DashboardInboxCards(props: {
  workspaceId: string
  superAdmin: boolean
  blocked: boolean
  reason: "status" | "mac" | null
}) {
  // `inboxService.list` is a page of 10 (the default `perPage`).
  const inboxes = useDashboardInboxes().slice(0, 10)
  return (
    <InboxCardList
      allowAddNew={props.superAdmin}
      blocked={props.blocked}
      inboxes={inboxes as never}
      reason={props.reason}
      workspaceId={props.workspaceId}
    />
  )
}

export function DashboardIndexPage() {
  const workspaceId = useWorkspaceId()
  return <Redirect href={`/space/${workspaceId}/dashboard/contacts`} />
}

/** The props both dashboards share (`contacts/page.tsx`, `conversations/page.tsx`). */
function useDashboardProps() {
  const workspaceId = useWorkspaceId()
  const { permissions, allWorkspaces } = useWorkspacePage()
  const inboxes = useDashboardInboxes()
  const isSuperAdmin = hasWorkspacePermission(permissions, "superAdmin")
  // `listAnalyticsFilterOptions`: known channels, by name.
  const filterInboxes = inboxes
    .flatMap((inbox) => {
      const channel = channelTypes.safeParse(inbox.channel)
      return channel.success
        ? [{ id: inbox.id, name: inbox.name, channel: channel.data }]
        : []
    })
    .sort((a, b) => a.name.localeCompare(b.name))
  // `resolveAdsDashboardChannels`: super admins, connected eligible channels.
  const adsChannels = isSuperAdmin
    ? adsEligibleChannelTypes.options.filter((channel) =>
        inboxes.some((inbox) => inbox.channel === channel),
      )
    : []
  const workspace = allWorkspaces.find((item) => item.id === workspaceId) as
    | { createdAt?: string }
    | undefined

  return {
    channelFilter: <AnalyticsChannelFilter options={filterInboxes} />,
    defaultSearchParams: {
      workspaceId,
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    },
    nav: <AnalyticsNav adsChannels={adsChannels} showGoogleAds={false} />,
    workspaceCreatedAt: workspace?.createdAt
      ? new Date(workspace.createdAt)
      : undefined,
  }
}

export function DashboardContactsPage() {
  return <ContactsDashboard {...useDashboardProps()} />
}

export function DashboardConversationsPage() {
  return (
    <div className="flex flex-col gap-4">
      <ConversationsDashboard {...useDashboardProps()} />
    </div>
  )
}

/** `dashboard/ads/*`: the ads domain is not in the Rust server yet (Q.D7). */
export function DashboardAdsPage() {
  return (
    <NotAvailablePage
      detail="Ads analytics is not available yet in this version."
      title="Ads"
    />
  )
}
