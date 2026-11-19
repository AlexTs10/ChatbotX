// page: app/space/[workspaceId]/(has-folder)/triggers/layout.tsx
// page: app/space/[workspaceId]/(has-folder)/triggers/page.tsx
// page: app/space/[workspaceId]/triggers/[id]/edit/page.tsx
// `resolveGuardedWorkspaceId(params, "superAdmin")` / `requireWorkspacePermission`
// (404 without super admin); the Rust page queries check it too.
import { folderTypes } from "@chatbotx.io/database/partials"
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@chatbotx.io/ui/components/ui/card"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet, useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { type ReactNode, Suspense } from "react"
import { FolderStoreProvider } from "@/features/folders/provider/folder-store-context"
import SharedFolderSlot from "@/features/folders/shared-folder-slot"
import { LockedEditorGuard } from "@/features/templates/components/locked-editor-guard"
import { findTriggerPage, getTriggers } from "@/features/triggers/queries"
import { getTriggersSearchParamsCache } from "@/features/triggers/schema/query"
import { TriggersTable } from "@/features/triggers/triggers-table"
import UpdateTriggerForm from "@/features/triggers/update-trigger-form"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import { useQueryPromise } from "@/lib/query/use-query-promise"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"

function useIds() {
  return useParams({ strict: false }) as { workspaceId: string; id?: string }
}

function SuperAdminGuard(props: { children: ReactNode }) {
  const { permissions } = useWorkspacePage()
  if (!hasWorkspacePermission(permissions, "superAdmin")) {
    return <NotFoundPage />
  }
  return props.children
}

export function TriggersLayout() {
  const { workspaceId } = useIds()
  return (
    <SuperAdminGuard>
      <FolderStoreProvider
        autoInitialize={true}
        folderType={folderTypes.enum.trigger}
        workspaceId={workspaceId}
      >
        <Suspense>
          <Outlet />
        </Suspense>
      </FolderStoreProvider>
    </SuperAdminGuard>
  )
}

export function TriggersPage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const searchParams = useSearchParams()
  const search = getTriggersSearchParamsCache.parse(
    Object.fromEntries(searchParams),
  )
  const input = { ...search, workspaceId }
  const promises = useQueryPromise(
    ["triggers", workspaceId, JSON.stringify(input)],
    () => Promise.all([getTriggers(input)]),
  )
  return (
    <Card>
      <CardHeader>
        <CardTitle className="font-bold text-xl">
          {t("triggers.title")}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <SharedFolderSlot
          folderType={folderTypes.enum.trigger}
          workspaceId={workspaceId}
        />
        <Suspense>
          <TriggersTable
            folderId={search.folderId}
            promises={promises}
            workspaceId={workspaceId}
          />
        </Suspense>
      </CardContent>
    </Card>
  )
}

function EditTrigger() {
  const { workspaceId, id = "" } = useIds()
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "triggerEditPage", workspaceId, id],
    queryFn: () => findTriggerPage({ workspaceId, id }),
  })
  if (!data) {
    return <NotFoundPage />
  }
  return (
    <LockedEditorGuard
      resourceId={id}
      resourceKind="trigger"
      workspaceId={workspaceId}
    >
      <UpdateTriggerForm
        storeSource={data.storeSource}
        trigger={data.trigger}
        workspaceId={workspaceId}
      />
    </LockedEditorGuard>
  )
}

export function EditTriggerPage() {
  return (
    <SuperAdminGuard>
      <Suspense>
        <EditTrigger />
      </Suspense>
    </SuperAdminGuard>
  )
}
