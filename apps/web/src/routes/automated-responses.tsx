// page: app/space/[workspaceId]/(has-folder)/automated-responses/layout.tsx
// page: app/space/[workspaceId]/(has-folder)/automated-responses/page.tsx
// page: app/space/[workspaceId]/(has-folder)/page-automated-responses/layout.tsx
// page: app/space/[workspaceId]/(has-folder)/page-automated-responses/page.tsx
// page: app/space/[workspaceId]/automated-responses/create/page.tsx
// page: app/space/[workspaceId]/automated-responses/[id]/edit/page.tsx
// page: app/space/[workspaceId]/page-automated-responses/create/page.tsx
// page: app/space/[workspaceId]/page-automated-responses/[id]/edit/page.tsx
// Keywords (inbound) and page keywords (outbound). Every layout is
// `resolveGuardedWorkspaceId(params, "superAdmin")` (404 otherwise).
import {
  type AutomatedResponseType,
  folderTypes,
  rootFolderId,
} from "@chatbotx.io/database/partials"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet, useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { type ReactNode, Suspense } from "react"
import { AutomatedResponsesTable } from "@/features/automated-response/automated-response-table"
import { CreateAutomatedResponseForm } from "@/features/automated-response/create-automated-response-form"
import EditAutomatedResponseForm from "@/features/automated-response/edit-automated-response-form"
import { KeywordsDescription } from "@/features/automated-response/keywords-description"
import { KeywordsTab } from "@/features/automated-response/keywords-tab"
import {
  findAutomatedResponse,
  listAutomatedResponses,
} from "@/features/automated-response/queries"
import { listAutomatedResponsesSearchParams } from "@/features/automated-response/schema/query"
import { FolderStoreProvider } from "@/features/folders/provider/folder-store-context"
import { WithFolderBar } from "@/features/folders/with-folder-bar"
import { LockedEditorGuard } from "@/features/templates/components/locked-editor-guard"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import { useQueryPromise } from "@/lib/query/use-query-promise"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"

const VARIANTS = {
  inbound: {
    basePath: "automated-responses",
    folderType: folderTypes.enum.automatedResponse,
  },
  outbound: {
    basePath: "page-automated-responses",
    folderType: folderTypes.enum.outboundAutomatedResponse,
  },
} as const

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

function FolderLayout(props: { type: AutomatedResponseType }) {
  const { workspaceId } = useIds()
  return (
    <SuperAdminGuard>
      <FolderStoreProvider
        folderType={VARIANTS[props.type].folderType}
        workspaceId={workspaceId}
      >
        <KeywordsTab />
        <KeywordsDescription type={props.type} />
        <Suspense>
          <Outlet />
        </Suspense>
      </FolderStoreProvider>
    </SuperAdminGuard>
  )
}

function ListPage(props: { type: AutomatedResponseType }) {
  const { workspaceId } = useIds()
  const searchParams = useSearchParams()
  const search = listAutomatedResponsesSearchParams.parse(
    Object.fromEntries(searchParams),
  )
  const input = {
    ...search,
    type: props.type,
    folderId: search.folderId ?? rootFolderId,
    workspaceId,
  }
  const promises = useQueryPromise(
    ["automatedResponses", workspaceId, JSON.stringify(input)],
    () => Promise.all([listAutomatedResponses(input)]),
  )
  const variant = VARIANTS[props.type]
  return (
    <WithFolderBar folderType={variant.folderType} workspaceId={workspaceId}>
      <Suspense>
        <AutomatedResponsesTable
          basePath={variant.basePath}
          promises={promises}
          type={props.type}
          workspaceId={workspaceId}
        />
      </Suspense>
    </WithFolderBar>
  )
}

function CreatePage(props: { type: AutomatedResponseType }) {
  const { workspaceId } = useIds()
  const searchParams = useSearchParams()
  return (
    <SuperAdminGuard>
      <CreateAutomatedResponseForm
        basePath={VARIANTS[props.type].basePath}
        folderId={searchParams.get("folderId")}
        type={props.type}
        workspaceId={workspaceId}
      />
    </SuperAdminGuard>
  )
}

function EditForm(props: { type: AutomatedResponseType }) {
  const { workspaceId, id = "" } = useIds()
  const { data: automatedResponse } = useSuspenseQuery({
    queryKey: ["spa", "automatedResponse", workspaceId, id, props.type],
    queryFn: () =>
      findAutomatedResponse({ workspaceId, id, type: props.type }),
  })
  if (!automatedResponse) {
    return <NotFoundPage />
  }
  return (
    <LockedEditorGuard
      resourceId={id}
      resourceKind="automatedResponse"
      workspaceId={workspaceId}
    >
      <EditAutomatedResponseForm
        automatedResponse={automatedResponse}
        type={props.type}
        workspaceId={workspaceId}
      />
    </LockedEditorGuard>
  )
}

function EditPage(props: { type: AutomatedResponseType }) {
  return (
    <SuperAdminGuard>
      <Suspense>
        <EditForm type={props.type} />
      </Suspense>
    </SuperAdminGuard>
  )
}

export const AutomatedResponsesLayout = () => <FolderLayout type="inbound" />
export const AutomatedResponsesPage = () => <ListPage type="inbound" />
export const CreateAutomatedResponsePage = () => <CreatePage type="inbound" />
export const EditAutomatedResponsePage = () => <EditPage type="inbound" />
export const PageAutomatedResponsesLayout = () => (
  <FolderLayout type="outbound" />
)
export const PageAutomatedResponsesPage = () => <ListPage type="outbound" />
export const CreatePageAutomatedResponsePage = () => (
  <CreatePage type="outbound" />
)
export const EditPageAutomatedResponsePage = () => <EditPage type="outbound" />
