// page: app/space/[workspaceId]/(has-folder)/sequences/layout.tsx
// page: app/space/[workspaceId]/(has-folder)/sequences/page.tsx
// `resolveGuardedWorkspaceId(params, "broadcast")`: 404 without the
// broadcast permission. Workspace membership is the workspace layout's check.
import { folderTypes } from "@chatbotx.io/database/partials"
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@chatbotx.io/ui/components/ui/card"
import { Outlet, useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { Suspense, useMemo } from "react"
import { FolderStoreProvider } from "@/features/folders/provider/folder-store-context"
import SharedFolderSlot from "@/features/folders/shared-folder-slot"
import { listSequences } from "@/features/sequences/queries"
import { listSequencesSearchParamsCache } from "@/features/sequences/schema/action"
import { SequencesTable } from "@/features/sequences/sequences-table"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"

function useWorkspaceId() {
  return (useParams({ strict: false }) as { workspaceId: string }).workspaceId
}

export function SequencesLayout() {
  const workspaceId = useWorkspaceId()
  const { permissions } = useWorkspacePage()
  if (!hasWorkspacePermission(permissions, "broadcast")) {
    return <NotFoundPage />
  }

  return (
    <FolderStoreProvider
      autoInitialize={true}
      folderType={folderTypes.enum.sequence}
      workspaceId={workspaceId}
    >
      <Suspense>
        <Outlet />
      </Suspense>
    </FolderStoreProvider>
  )
}

export function SequencesPage() {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  const searchParams = useSearchParams()
  const search = listSequencesSearchParamsCache.parse(
    Object.fromEntries(searchParams),
  )
  const key = searchParams.toString()

  // biome-ignore lint/correctness/useExhaustiveDependencies: the search string is the key
  const promises = useMemo(
    () => Promise.all([listSequences({ ...search, workspaceId })]),
    [key, workspaceId],
  )

  return (
    <Card>
      <CardHeader>
        <CardTitle className="font-bold text-xl">
          {t("sequences.title")}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <SharedFolderSlot
          folderType={folderTypes.enum.sequence}
          workspaceId={workspaceId}
        />
        {/* The table suspends here, so the page keeps its memoized promise. */}
        <Suspense>
          <SequencesTable promises={promises} workspaceId={workspaceId} />
        </Suspense>
      </CardContent>
    </Card>
  )
}
