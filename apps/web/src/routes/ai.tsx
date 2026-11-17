// page: app/space/[workspaceId]/(ai)/layout.tsx
// page: app/space/[workspaceId]/(ai)/ai-agents/page.tsx
// page: app/space/[workspaceId]/(ai)/ai-files/page.tsx
// page: app/space/[workspaceId]/(ai)/ai-files/[aiFileId]/page.tsx
// page: app/space/[workspaceId]/(ai)/ai-files/test-query/page.tsx
// page: app/space/[workspaceId]/(ai)/ai-functions/page.tsx
// page: app/space/[workspaceId]/(ai)/ai-mcp-servers/page.tsx
// `resolveGuardedWorkspaceId(params, "flows")`: 404 without the flows
// permission. The agent action options are the page loader `pages.aiAgents`.
import type { WorkspaceModel } from "@chatbotx.io/database/types"
import { isNumericId } from "@chatbotx.io/utils"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet, useParams } from "@tanstack/react-router"
import { ArrowLeftIcon } from "lucide-react"
import Link from "next/link"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { Suspense } from "react"
import { AIAgentsTable } from "@/features/ai-agents/ai-agent-table"
import type { AIAgentActionOptions } from "@/features/ai-agents/components/ai-actions-field"
import { listAIAgents } from "@/features/ai-agents/queries"
import { listAIAgentsRequest } from "@/features/ai-agents/schema/query"
import AIFilesTable from "@/features/ai-files/ai-files-table"
import { AIFileDetail } from "@/features/ai-files/components/ai-file-detail"
import { TestQueryPage } from "@/features/ai-files/components/test-query/test-query-page"
import { getAIFile, listAIFiles } from "@/features/ai-files/queries"
import { AIFunctionsTable } from "@/features/ai-functions/ai-functions-table"
import { listAIFunctions } from "@/features/ai-functions/queries"
import { AITab } from "@/features/ai-hub/ai-hub-breadcrumb"
import { AIMcpServersTable } from "@/features/ai-mcp-servers/ai-mcp-servers-table"
import { listAIMcpServers } from "@/features/ai-mcp-servers/queries"
import { AIToolsStoreProvider } from "@/features/ai-tools/provider/ai-tools-store-context"
import { listIntegrationOpenaiCompatible } from "@/features/integration-openai-compatible/queries"
import { NoAIIntegrationFound } from "@/features/integrations/components/no-ai-integration-found"
import { hasAIIntegration } from "@/features/integrations/queries/get-ai-integrations"
import { LockedIdsProvider } from "@/features/templates/components/lock-context"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import { useQueryPromise } from "@/lib/query/use-query-promise"
import { loadPage } from "@/lib/server-action"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"

function useWorkspaceId() {
  return (useParams({ strict: false }) as { workspaceId: string }).workspaceId
}

export function AILayout() {
  const workspaceId = useWorkspaceId()
  const { permissions } = useWorkspacePage()
  const allowed = hasWorkspacePermission(permissions, "flows")
  const { data: hasIntegration } = useSuspenseQuery({
    queryKey: ["spa", "hasAIIntegration", workspaceId],
    queryFn: () => (allowed ? hasAIIntegration(workspaceId) : false),
  })
  if (!allowed) {
    return <NotFoundPage />
  }
  if (!hasIntegration) {
    return <NoAIIntegrationFound workspaceId={workspaceId} />
  }

  return (
    <AIToolsStoreProvider workspaceId={workspaceId}>
      <Outlet />
    </AIToolsStoreProvider>
  )
}

export function AIAgentsPage() {
  const workspaceId = useWorkspaceId()
  const searchParams = useSearchParams()
  const { allWorkspaces } = useWorkspacePage()
  const workspace = allWorkspaces.find(
    (item) => item.id === workspaceId,
  ) as unknown as WorkspaceModel
  const promises = useQueryPromise(
    ["aiAgents", workspaceId, searchParams.toString()],
    () =>
      Promise.all([
        listAIAgents({
          workspaceId,
          ...listAIAgentsRequest.parse(Object.fromEntries(searchParams)),
        }),
        listIntegrationOpenaiCompatible({ workspaceId }),
        Promise.resolve(workspace),
        loadPage<AIAgentActionOptions>("aiAgents", { workspaceId }),
      ] as const),
  )

  return (
    <div className="space-y-6">
      <AITab />

      <Suspense>
        <AIAgentsTable promises={promises} workspaceId={workspaceId} />
      </Suspense>
    </div>
  )
}

export function AIFilesPage() {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(["aiFiles", workspaceId], () =>
    Promise.all([listAIFiles({ workspaceId })] as const),
  )

  return (
    <div className="space-y-6">
      <AITab />

      <Suspense>
        <AIFilesTable promises={promises} />
      </Suspense>
    </div>
  )
}

export function AIFileDetailPage() {
  const { workspaceId, aiFileId } = useParams({ strict: false }) as {
    workspaceId: string
    aiFileId: string
  }
  // A non-numeric id is a 404, not a failed bigint cast.
  const { data: file } = useSuspenseQuery({
    queryKey: ["spa", "aiFile", workspaceId, aiFileId],
    queryFn: () =>
      isNumericId(aiFileId) ? getAIFile({ workspaceId, id: aiFileId }) : null,
  })
  if (!file) {
    return <NotFoundPage />
  }

  return (
    <div className="space-y-6">
      <AITab />
      <LockedIdsProvider
        resourceIds={[aiFileId]}
        resourceKind="aiFile"
        workspaceId={workspaceId}
      >
        <AIFileDetail aiFileId={aiFileId} workspaceId={workspaceId} />
      </LockedIdsProvider>
    </div>
  )
}

export function AIFilesTestQueryPage() {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  const {
    data: { data: files },
  } = useSuspenseQuery({
    queryKey: ["spa", "aiFiles", workspaceId],
    queryFn: () => Promise.all([listAIFiles({ workspaceId })] as const),
    select: ([list]) => list,
  })

  return (
    <div className="space-y-6">
      <AITab />
      <div className="space-y-1">
        <Link
          className="inline-flex items-center gap-1 text-muted-foreground text-sm hover:underline"
          href={`/space/${workspaceId}/ai-files`}
        >
          <ArrowLeftIcon className="h-4 w-4" />
          {t("aiFiles.testQuery.backToKnowledge")}
        </Link>
        <h1 className="font-semibold text-xl">
          {t("aiFiles.testQuery.title")}
        </h1>
      </div>
      <Suspense>
        <TestQueryPage files={files} workspaceId={workspaceId} />
      </Suspense>
    </div>
  )
}

export function AIFunctionsPage() {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(["aiFunctions", workspaceId], () =>
    Promise.all([listAIFunctions({ workspaceId })] as const),
  )

  return (
    <div className="space-y-6">
      <AITab />

      <Suspense>
        <AIFunctionsTable promises={promises} workspaceId={workspaceId} />
      </Suspense>
    </div>
  )
}

export function AIMcpServersPage() {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(["aiMcpServers", workspaceId], () =>
    Promise.all([listAIMcpServers({ workspaceId })] as const),
  )

  return (
    <div className="space-y-6">
      <AITab />
      <Suspense>
        <AIMcpServersTable promises={promises} workspaceId={workspaceId} />
      </Suspense>
    </div>
  )
}
