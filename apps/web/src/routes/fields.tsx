// page: app/space/[workspaceId]/(has-folder)/tags/layout.tsx
// page: app/space/[workspaceId]/(has-folder)/tags/page.tsx
// page: app/space/[workspaceId]/(has-folder)/custom-fields/layout.tsx
// page: app/space/[workspaceId]/(has-folder)/custom-fields/page.tsx
import { folderTypes, rootFolderId } from "@chatbotx.io/database/partials"
import { Outlet, useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { Suspense, useMemo } from "react"
import { AppTab } from "@/components/app-tab"
import { AccountFieldsCard } from "@/features/bot-fields/account-fields-card"
import {
  ACCOUNT_FIELDS_HARD_CAP,
  listBotFieldsRSC,
} from "@/features/bot-fields/queries"
import { CustomFieldsTable } from "@/features/custom-fields/custom-field-table"
import { listCustomFieldsRSC } from "@/features/custom-fields/queries"
import { listCustomFieldsSearchParams } from "@/features/custom-fields/schema/query"
import { FolderStoreProvider } from "@/features/folders/provider/folder-store-context"
import { WithFolderBar } from "@/features/folders/with-folder-bar"
import { listTags } from "@/features/tags/queries"
import { listTagsSearchParamsCache } from "@/features/tags/schema/query"
import { TagsTable } from "@/features/tags/tags-table"

function useWorkspaceId() {
  return (useParams({ strict: false }) as { workspaceId: string }).workspaceId
}

// The flows / tags / custom-fields layouts differ only in the active tab.
function FieldsLayout(props: { active: "tags" | "custom-fields" }) {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  const tab = (label: string, segment: string) => ({
    label,
    href: `/space/${workspaceId}/${segment}`,
    isActive: segment === props.active,
  })

  return (
    <FolderStoreProvider
      folderType={props.active === "tags" ? "tag" : "customField"}
      workspaceId={workspaceId}
    >
      <AppTab
        tabs={[
          tab(t("flows.title"), "flows"),
          tab(t("tags.title"), "tags"),
          tab(t("customFields.title"), "custom-fields"),
          tab(t("emailTopics.title"), "email-topics"),
          tab(t("errorLogs.title"), "error-logs"),
        ]}
      />
      <Outlet />
    </FolderStoreProvider>
  )
}

export const TagsLayout = () => <FieldsLayout active="tags" />
export const CustomFieldsLayout = () => <FieldsLayout active="custom-fields" />

export function TagsPage() {
  const workspaceId = useWorkspaceId()
  const searchParams = useSearchParams()
  const search = listTagsSearchParamsCache.parse(
    Object.fromEntries(searchParams),
  )
  const folderId = search.folderId ?? rootFolderId
  const key = searchParams.toString()

  // biome-ignore lint/correctness/useExhaustiveDependencies: the search string is the key
  const promises = useMemo(
    () => Promise.all([listTags({ ...search, folderId, workspaceId })]),
    [key, folderId, workspaceId],
  )

  return (
    <WithFolderBar folderType={folderTypes.enum.tag} workspaceId={workspaceId}>
      <Suspense>
        <TagsTable
          folderId={folderId}
          promises={promises}
          workspaceId={workspaceId}
        />
      </Suspense>
    </WithFolderBar>
  )
}

export function CustomFieldsPage() {
  const workspaceId = useWorkspaceId()
  const searchParams = useSearchParams()
  const search = listCustomFieldsSearchParams.parse(
    Object.fromEntries(searchParams),
  )
  const folderId = search.folderId ?? rootFolderId
  const key = searchParams.toString()

  // biome-ignore lint/correctness/useExhaustiveDependencies: the search string is the key
  const customFieldsPromises = useMemo(
    () =>
      Promise.all([listCustomFieldsRSC({ ...search, workspaceId, folderId })]),
    [key, folderId, workspaceId],
  )
  const botFieldsPromises = useMemo(
    () =>
      Promise.all([
        listBotFieldsRSC({
          workspaceId,
          folderId,
          page: 1,
          perPage: ACCOUNT_FIELDS_HARD_CAP,
        }),
      ]),
    [folderId, workspaceId],
  )

  return (
    <WithFolderBar
      folderType={folderTypes.enum.customField}
      workspaceId={workspaceId}
    >
      <div className="flex flex-col gap-6">
        <Suspense>
          <CustomFieldsTable
            folderId={folderId}
            promises={customFieldsPromises}
            workspaceId={workspaceId}
          />
        </Suspense>
        <Suspense>
          <AccountFieldsCard
            folderId={folderId}
            promises={botFieldsPromises}
            workspaceId={workspaceId}
          />
        </Suspense>
      </div>
    </WithFolderBar>
  )
}
