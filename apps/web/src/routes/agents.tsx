// page: app/space/[workspaceId]/agents/page.tsx
// Membership only (`assertCurrentUserCanAccessChatbot`, the workspace
// layout's check).
import { useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { Suspense } from "react"
import { listWorkspaceMembers } from "@/features/workspace-members/queries"
import { getWorkspaceMembersSearchParamsCache } from "@/features/workspace-members/schema/query"
import { WorkspaceMembersTable } from "@/features/workspace-members/workspace-members-table"
import { useQueryPromise } from "@/lib/query/use-query-promise"

export function AgentsPage() {
  const { workspaceId } = useParams({ strict: false }) as {
    workspaceId: string
  }
  const searchParams = useSearchParams()
  const input = {
    ...getWorkspaceMembersSearchParamsCache.parse(
      Object.fromEntries(searchParams),
    ),
    workspaceId,
  }
  const promises = useQueryPromise(
    ["workspaceMembers", workspaceId, JSON.stringify(input)],
    () => Promise.all([listWorkspaceMembers(input)] as const),
  )
  return (
    <Suspense>
      {/* ponytail: no quota in Rust (as `SettingsAdminsPage`), so never at the limit. */}
      <WorkspaceMembersTable promises={promises} teamMembersAtLimit={false} />
    </Suspense>
  )
}
