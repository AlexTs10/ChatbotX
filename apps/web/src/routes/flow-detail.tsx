// page: app/(no-sidebar)/space/[workspaceId]/layout.tsx
// page: app/(no-sidebar)/space/[workspaceId]/flows/[id]/page.tsx
// page: app/(no-sidebar)/space/[workspaceId]/flows/[id]/analytics/page.tsx
// The server part (membership, "flows" permission, flow + draft, publish
// state, template lock, target country) is the Rust page loader `pages.flow`.
import { useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import type { ComponentProps } from "react"
import { CouponTopicStoreProvider } from "@/features/coupons/provider/coupon-topic-store-context"
import type { SmartDelayNodeStats } from "@/features/flows/analytics/smart-delay-node-stats"
import { FlowAnalytics } from "@/features/flows/flow-analytics"
import { FlowDetail } from "@/features/flows/flow-detail"
import { FlowTemplateStoreProvider } from "@/features/flows/react-flow/stores/flow-template-store-provider"
import { loadPage, serverQuery } from "@/lib/server-action"

export function FlowDetailPage() {
  const { workspaceId, id } = useParams({ strict: false }) as {
    workspaceId: string
    id: string
  }
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "flow", workspaceId, id],
    queryFn: () =>
      loadPage<ComponentProps<typeof FlowDetail>>("flow", { workspaceId, id }),
  })

  return (
    <CouponTopicStoreProvider autoInitialize={false} workspaceId={workspaceId}>
      <div className="flex h-screen w-screen flex-col">
        <FlowDetail {...data} />
      </div>
    </CouponTopicStoreProvider>
  )
}

/** The flow and its draft come from `pages.flow` (the same `flows`
 * permission and 404s); the stats from the `flowAnalyticsStats` query. */
export function FlowAnalyticsPage() {
  const { workspaceId, id } = useParams({ strict: false }) as {
    workspaceId: string
    id: string
  }
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "flow", workspaceId, id],
    queryFn: () =>
      loadPage<ComponentProps<typeof FlowDetail>>("flow", { workspaceId, id }),
  })
  const { data: analytics } = useSuspenseQuery({
    queryKey: ["spa", "flowAnalyticsStats", workspaceId, id],
    queryFn: () =>
      serverQuery<{
        stats: ComponentProps<typeof FlowAnalytics>["stats"]
        smartDelayStats: Record<string, SmartDelayNodeStats>
      }>("flowAnalyticsStats", workspaceId, [workspaceId, { id }]),
  })

  return (
    <div className="flex h-screen w-screen flex-col">
      <FlowTemplateStoreProvider
        openaiCompatibleIntegrations={data.openaiCompatibleIntegrations}
        workspaceId={workspaceId}
      >
        <FlowAnalytics
          flow={data.flow}
          flowVersion={data.flowVersion}
          smartDelayStats={analytics.smartDelayStats}
          stats={analytics.stats}
        />
      </FlowTemplateStoreProvider>
    </div>
  )
}
