// page: app/space/[workspaceId]/layout.tsx
// page: app/space/[workspaceId]/page.tsx
// The server part (membership, quota, token refresh errors, call-capable
// channel) is the Rust page loader `pages.workspace` (P4 spec §5.2); the
// pure gates (landing segment, realtime gates) run here as in the builder.

import type { WorkspaceMemberPermissions } from "@chatbotx.io/database/partials"
import {
  SidebarInset,
  SidebarMobileHandle,
  SidebarProvider,
  SidebarTrigger,
} from "@chatbotx.io/ui/components/ui/sidebar"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet, useParams } from "@tanstack/react-router"
import { AppSidebar } from "@/components/app-sidebar"
import { ExpiredBanner } from "@/components/expired-banner"
import type { QuotaSummary } from "@/components/nav-usage"
import {
  NavigationPendingBoundary,
  NavigationPendingProvider,
} from "@/components/navigation-pending"
import { RefreshOnNavigation } from "@/components/refresh-on-navigation"
import { ScheduledDeletionBanner } from "@/components/scheduled-deletion-banner"
import { SupportAccessBanner } from "@/components/support-access-banner"
import { TokenRefreshErrorDialog } from "@/components/token-refresh-error-dialog"
import { WorkspaceDeletionTabSync } from "@/components/workspace-deletion-tab-sync"
import { WorkspaceRealtimeShell } from "@/components/workspace-realtime-shell"
import { isCloud } from "@/env"
import { AnalyticsApiProvider } from "@/features/analytics/components/analytics-api-provider"
import { CouponTopicStoreProvider } from "@/features/coupons/provider/coupon-topic-store-context"
import type { WorkspaceResource } from "@/features/workspaces/schema/resource"
import {
  hasWorkspacePermission,
  resolveWorkspaceLandingSegment,
} from "@/lib/auth/permission-routes"
import { loadPage } from "@/lib/server-action"
import type { TokenRefreshErrorIntegration } from "@/lib/server-types"
import { resolveWorkspaceRealtimeGates } from "@/lib/workspace/resolve-workspace-realtime-gates"
import { workspaceSettingsGeneralPath } from "@/lib/workspace/settings-paths"
import { NotFoundPage } from "@/routes/root"
import { Redirect } from "@/shims/redirect"

export type WorkspacePageData = {
  allWorkspaces: WorkspaceResource[]
  permissions: WorkspaceMemberPermissions
  isSupportSession: boolean
  scheduledForDeletion: boolean
  supportAccessUntil: string | null
  consentRequired: boolean
  canAccessManage: boolean
  isSuperAdmin: boolean
  blocked: boolean
  blockReason: "status" | "mac" | null
  quota: QuotaSummary
  tokenRefreshErrors: TokenRefreshErrorIntegration[]
  hasCallCapableChannel: boolean
}

export const workspaceQuery = (workspaceId: string) => ({
  queryKey: ["spa", "workspace", workspaceId],
  queryFn: () => loadPage<WorkspacePageData>("workspace", { workspaceId }),
})

export function useWorkspacePage() {
  const { workspaceId } = useParams({ strict: false }) as {
    workspaceId: string
  }
  return useSuspenseQuery(workspaceQuery(workspaceId)).data
}

export function WorkspaceLayout() {
  const { workspaceId } = useParams({ strict: false }) as {
    workspaceId: string
  }
  const data = useWorkspacePage()
  const cloud = isCloud()
  const realtimeGates = resolveWorkspaceRealtimeGates({
    permissions: data.permissions,
    isSupportSession: data.isSupportSession,
    scheduledForDeletion: data.scheduledForDeletion,
    cloud,
    blocked: data.blocked,
    hasCallCapableChannel: data.hasCallCapableChannel,
  })
  const defaultOpen = document.cookie.includes("sidebar_state=true")

  // The deletion gate of `enforceWorkspaceNotScheduledForDeletionFromRequest`:
  // a workspace pending deletion only opens its general settings.
  const settingsPath = workspaceSettingsGeneralPath(workspaceId)
  if (
    data.scheduledForDeletion &&
    hasWorkspacePermission(data.permissions, "superAdmin") &&
    !window.location.pathname.startsWith(settingsPath)
  ) {
    return <Redirect href={settingsPath} />
  }

  return (
    <NavigationPendingProvider>
      <SidebarProvider
        className="has-data-full-bleed:h-svh"
        defaultOpen={defaultOpen}
      >
        <AppSidebar
          allWorkspaces={data.allWorkspaces}
          callHistoryNavVisible={realtimeGates.callHistoryNavVisible}
          canAccessManage={data.canAccessManage}
          isSuperAdmin={data.isSuperAdmin}
          permissions={data.permissions}
          quota={data.quota}
          scheduledForDeletion={data.scheduledForDeletion}
          workspaceId={workspaceId}
        />
        <SidebarInset>
          <main className="flex min-h-0 min-w-0 flex-1 flex-col gap-4 p-4 md:p-6">
            <WorkspaceDeletionTabSync
              scheduledForDeletion={data.scheduledForDeletion}
              workspaceId={workspaceId}
            />
            <ScheduledDeletionBanner scheduled={data.scheduledForDeletion} />
            {data.isSupportSession && (
              <SupportAccessBanner
                consentRequired={data.consentRequired}
                supportAccessUntil={
                  data.supportAccessUntil
                    ? new Date(data.supportAccessUntil)
                    : null
                }
              />
            )}
            {!data.scheduledForDeletion && (
              <RefreshOnNavigation workspaceId={workspaceId} />
            )}
            <ExpiredBanner
              blocked={cloud && data.blocked}
              reason={data.blockReason}
            />
            <TokenRefreshErrorDialog
              errors={data.tokenRefreshErrors}
              workspaceId={workspaceId}
            />
            <AnalyticsApiProvider>
              <CouponTopicStoreProvider
                autoInitialize={false}
                workspaceId={workspaceId}
              >
                <WorkspaceRealtimeShell
                  callHistoryEnabled={realtimeGates.callHistoryEnabled}
                  callingEnabled={realtimeGates.callingEnabled}
                  realtimeEnabled={realtimeGates.realtimeEnabled}
                >
                  <NavigationPendingBoundary>
                    <Outlet />
                  </NavigationPendingBoundary>
                </WorkspaceRealtimeShell>
              </CouponTopicStoreProvider>
            </AnalyticsApiProvider>
          </main>
          <SidebarTrigger className="absolute -inset-s-2 top-3 z-10 hidden border md:inline-flex" />
          <SidebarMobileHandle />
        </SidebarInset>
      </SidebarProvider>
    </NavigationPendingProvider>
  )
}

/** `app/space/[workspaceId]/page.tsx`: land on the first section the member can open. */
export function WorkspaceIndexPage() {
  const { workspaceId } = useParams({ strict: false }) as {
    workspaceId: string
  }
  const data = useWorkspacePage()
  const segment = resolveWorkspaceLandingSegment(data.permissions)
  if (!segment) {
    return <NotFoundPage />
  }
  return <Redirect href={`/space/${workspaceId}/${segment}`} />
}
