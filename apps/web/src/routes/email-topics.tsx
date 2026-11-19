// page: app/space/[workspaceId]/(has-folder)/email-topics/layout.tsx
// page: app/space/[workspaceId]/(has-folder)/email-topics/page.tsx
// Membership is the workspace layout's check (the builder layout has no
// permission guard).
import { folderTypes, rootFolderId } from "@chatbotx.io/database/partials"
import { Outlet, useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { Suspense } from "react"
import { AppTab } from "@/components/app-tab"
import { EmailTopicsTable } from "@/features/email-topics/email-topics-table"
import { listEmailTopicsRSC } from "@/features/email-topics/queries"
import { listEmailTopicsSearchParamsCache } from "@/features/email-topics/schema/query"
import { FolderStoreProvider } from "@/features/folders/provider/folder-store-context"
import { WithFolderBar } from "@/features/folders/with-folder-bar"
import { useQueryPromise } from "@/lib/query/use-query-promise"

function useWorkspaceId() {
  return (useParams({ strict: false }) as { workspaceId: string }).workspaceId
}

export function EmailTopicsLayout() {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  const tab = (key: string, path: string) => ({
    label: t(key),
    href: `/space/${workspaceId}/${path}`,
    isActive: path === "email-topics",
  })
  return (
    <FolderStoreProvider folderType="emailTopic" workspaceId={workspaceId}>
      <AppTab
        tabs={[
          tab("flows.title", "flows"),
          tab("tags.title", "tags"),
          tab("customFields.title", "custom-fields"),
          tab("emailTopics.title", "email-topics"),
          tab("errorLogs.title", "error-logs"),
        ]}
      />
      <Suspense>
        <Outlet />
      </Suspense>
    </FolderStoreProvider>
  )
}

export function EmailTopicsPage() {
  const workspaceId = useWorkspaceId()
  const searchParams = useSearchParams()
  const search = listEmailTopicsSearchParamsCache.parse(
    Object.fromEntries(searchParams),
  )
  const folderId = search.folderId ?? rootFolderId
  const input = { ...search, folderId, workspaceId }
  const promises = useQueryPromise(
    ["emailTopics", workspaceId, JSON.stringify(input)],
    () => Promise.all([listEmailTopicsRSC(input)]),
  )
  return (
    <WithFolderBar
      folderType={folderTypes.enum.emailTopic}
      workspaceId={workspaceId}
    >
      <Suspense>
        <EmailTopicsTable
          folderId={folderId}
          promises={promises}
          workspaceId={workspaceId}
        />
      </Suspense>
    </WithFolderBar>
  )
}
