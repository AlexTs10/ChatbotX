// page: app/(no-sidebar)/space/[workspaceId]/layout.tsx
// page: app/(no-sidebar)/space/[workspaceId]/flows/[id]/page.tsx
// The server part (membership, "flows" permission, flow + draft, publish
// state, template lock, target country) is the Rust page loader `pages.flow`.
import { useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import type { ComponentProps } from "react"
import { CouponTopicStoreProvider } from "@/features/coupons/provider/coupon-topic-store-context"
import { FlowDetail } from "@/features/flows/flow-detail"
import { loadPage } from "@/lib/server-action"

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
