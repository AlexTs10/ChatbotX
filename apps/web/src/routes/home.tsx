// page: app/page.tsx
// The server part (workspace list, quota, first-workspace creation) is the
// Rust page loader `pages.home` (P4 spec §5.2).
import { useSuspenseQuery } from "@tanstack/react-query"
import { ExpiredBanner } from "@/components/expired-banner"
import { WorkspaceDeletionPendingToast } from "@/components/workspace-deletion-pending-toast"
import { isCloud } from "@/env"
import { AccountRail } from "@/features/workspaces/components/account-rail"
import { DeletedWorkspaces } from "@/features/workspaces/components/deleted-workspaces"
import WorkspacesList from "@/features/workspaces/components/workspaces-list"
import type { WorkspaceResource } from "@/features/workspaces/schema/resource"
import type { QuotaMetric } from "@/lib/quota-metrics"
import { loadPage } from "@/lib/server-action"
import { Redirect } from "@/shims/redirect"

type HomePage = {
  user: { id: string; name: string | null; email: string; image: string | null }
  /** Set when the loader just created the user's first workspace. */
  createdWorkspaceId: string | null
  workspaces: WorkspaceResource[]
  ownerWorkspaceIds: string[]
  superAdminWorkspaceIds: string[]
  blocked: boolean
  reason: "status" | "mac" | null
  isAtLimit: boolean
  canAccessManage: boolean
  isPlatformContext: boolean
  isSuperAdmin: boolean
  metrics: QuotaMetric[]
  planName: string | null
  planStatus: string | null
  trialEndsAt: string | null
}

export const homeQuery = {
  queryKey: ["spa", "home"],
  queryFn: () => loadPage<HomePage>("home"),
}

export function MainPage() {
  const { data } = useSuspenseQuery(homeQuery)
  if (data.createdWorkspaceId) {
    return <Redirect href={`/space/${data.createdWorkspaceId}/dashboard`} />
  }

  const cloud = isCloud()
  const userInfo = {
    name: data.user.name,
    email: data.user.email,
    image: data.user.image,
  }

  return (
    <div className="mx-auto w-full max-w-6xl px-6">
      <WorkspaceDeletionPendingToast />
      <ExpiredBanner blocked={cloud && data.blocked} reason={data.reason} />
      <div className="flex flex-col gap-8 py-8 md:flex-row md:items-start">
        <AccountRail
          canAccessManage={data.canAccessManage}
          isPlatformContext={data.isPlatformContext}
          isSuperAdmin={data.isSuperAdmin}
          metrics={data.metrics}
          planName={data.planName}
          planStatus={data.planStatus}
          trialEndsAt={data.trialEndsAt}
          user={userInfo}
        />

        <div className="min-w-0 flex-1 space-y-8">
          <WorkspacesList
            blocked={cloud && data.blocked}
            isAtLimit={data.isAtLimit}
            ownerWorkspaceIds={data.ownerWorkspaceIds}
            reason={data.reason}
            superAdminWorkspaceIds={data.superAdminWorkspaceIds}
            user={userInfo}
            workspaces={data.workspaces}
          />
          <DeletedWorkspaces ownerId={data.user.id} />
        </div>
      </div>
    </div>
  )
}
