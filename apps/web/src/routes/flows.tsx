// page: app/space/[workspaceId]/(has-folder)/flows/layout.tsx
// page: app/space/[workspaceId]/(has-folder)/flows/page.tsx
import { folderTypes, rootFolderId } from "@chatbotx.io/database/partials"
import { Outlet, useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { Suspense, useMemo } from "react"
import { AppTab } from "@/components/app-tab"
import { FlowsTable } from "@/features/flows/flows-table"
import { listFlowsRSC } from "@/features/flows/queries"
import { listFlowsSearchParams } from "@/features/flows/schema/query"
import { FolderStoreProvider } from "@/features/folders/provider/folder-store-context"
import { WithFolderBar } from "@/features/folders/with-folder-bar"

export function FlowsLayout() {
  const t = useTranslations()
  const { workspaceId } = useParams({ strict: false }) as {
    workspaceId: string
  }

  return (
    <FolderStoreProvider folderType="flow" workspaceId={workspaceId}>
      <AppTab
        tabs={[
          {
            label: t("flows.title"),
            href: `/space/${workspaceId}/flows`,
            isActive: true,
          },
          {
            label: t("tags.title"),
            href: `/space/${workspaceId}/tags`,
            isActive: false,
          },
          {
            label: t("customFields.title"),
            href: `/space/${workspaceId}/custom-fields`,
            isActive: false,
          },
          {
            label: t("emailTopics.title"),
            href: `/space/${workspaceId}/email-topics`,
            isActive: false,
          },
          {
            label: t("errorLogs.title"),
            href: `/space/${workspaceId}/error-logs`,
            isActive: false,
          },
        ]}
      />
      <Outlet />
    </FolderStoreProvider>
  )
}

export function FlowsPage() {
  const { workspaceId } = useParams({ strict: false }) as {
    workspaceId: string
  }
  const searchParams = useSearchParams()
  const search = listFlowsSearchParams.parse(Object.fromEntries(searchParams))
  const folderId = search.folderId ?? rootFolderId
  const key = searchParams.toString()

  // biome-ignore lint/correctness/useExhaustiveDependencies: the search string is the key
  const promises = useMemo(
    () => Promise.all([listFlowsRSC({ ...search, folderId, workspaceId })]),
    [key, folderId, workspaceId],
  )

  return (
    <WithFolderBar folderType={folderTypes.enum.flow} workspaceId={workspaceId}>
      <Suspense>
        <FlowsTable
          folderId={folderId}
          promises={promises}
          workspaceId={workspaceId}
        />
      </Suspense>
    </WithFolderBar>
  )
}
