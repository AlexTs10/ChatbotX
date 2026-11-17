// page: app/space/[workspaceId]/polls/page.tsx
// page: app/space/[workspaceId]/questionnaires/page.tsx
// page: app/space/[workspaceId]/reflinks/page.tsx
// page: app/space/[workspaceId]/qr-codes/page.tsx
// page: app/space/[workspaceId]/minigames/page.tsx
// page: app/space/[workspaceId]/pipelines/layout.tsx
// page: app/space/[workspaceId]/pipelines/page.tsx
// The workspace tools lists. Membership is the workspace layout's check;
// pipelines also need contacts access (`requireContactsAccess`).
import { Outlet, useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { type ReactNode, Suspense, useMemo } from "react"
import { AppBreadcrumb } from "@/components/app-breadcrumb"
import { MinigamesTable } from "@/features/minigames/minigames-table"
import { listMinigames } from "@/features/minigames/queries"
import { listMinigamesSearchParamsCache } from "@/features/minigames/schema/query"
import { PipelinesList } from "@/features/pipelines/components/list/pipelines-list"
import { PollsTable } from "@/features/polls/components/polls-table"
import { listPolls } from "@/features/polls/queries"
import { listPollsSearchParamsCache } from "@/features/polls/schema/query"
import { QrCodesTable } from "@/features/qr-codes/qr-codes-table"
import { listQrCodes } from "@/features/qr-codes/queries"
import { listQrCodesSearchParamsCache } from "@/features/qr-codes/schema/query"
import { QuestionnairesTable } from "@/features/questionnaires/components/questionnaires-table"
import { listQuestionnaires } from "@/features/questionnaires/queries"
import { listQuestionnairesSearchParamsCache } from "@/features/questionnaires/schema/query"
import { listReflinks } from "@/features/reflinks/queries"
import { ReflinksTable } from "@/features/reflinks/reflinks-table"
import { listReflinksSearchParamsCache } from "@/features/reflinks/schema/query"
import { hasContactsAccess } from "@/lib/auth/permission-routes"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"

function useWorkspaceId() {
  return (useParams({ strict: false }) as { workspaceId: string }).workspaceId
}

/**
 * The list promise of a page, made once per search string (the table
 * suspends below, so the page keeps its memoized promise).
 */
function useListPromise<S, T>(
  // `parse` is overloaded (the last one takes a Promise): `all` gives the type.
  cache: { all: () => S; parse: (input: Record<string, string>) => unknown },
  load: (input: S & { workspaceId: string }) => Promise<T>,
) {
  const workspaceId = useWorkspaceId()
  const searchParams = useSearchParams()
  const key = searchParams.toString()
  // biome-ignore lint/correctness/useExhaustiveDependencies: the search string is the key
  return useMemo(
    () =>
      Promise.all([
        load({
          ...(cache.parse(Object.fromEntries(searchParams)) as S),
          workspaceId,
        }),
      ]) as Promise<[T]>,
    [key, workspaceId],
  )
}

/** The breadcrumb + table frame of a tools list page. */
function ToolsListPage(props: {
  title: string
  fallback?: ReactNode
  children: ReactNode
}) {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb
        items={[
          { label: t("tools.title"), href: `/space/${workspaceId}/tools` },
          { label: props.title, href: "" },
        ]}
      />
      <Suspense fallback={props.fallback}>{props.children}</Suspense>
    </div>
  )
}

export function PollsPage() {
  const t = useTranslations()
  const promises = useListPromise(listPollsSearchParamsCache, listPolls)
  return (
    <ToolsListPage
      fallback={<div>{t("actions.loading")}</div>}
      title={t("pollManager.title")}
    >
      <PollsTable promises={promises} workspaceId={useWorkspaceId()} />
    </ToolsListPage>
  )
}

export function QuestionnairesPage() {
  const t = useTranslations()
  const promises = useListPromise(
    listQuestionnairesSearchParamsCache,
    listQuestionnaires,
  )
  return (
    <ToolsListPage
      fallback={<div>{t("actions.loading")}</div>}
      title={t("questionnaires.title")}
    >
      <QuestionnairesTable
        promises={promises as never}
        workspaceId={useWorkspaceId()}
      />
    </ToolsListPage>
  )
}

export function ReflinksPage() {
  const t = useTranslations()
  const promises = useListPromise(listReflinksSearchParamsCache, listReflinks)
  return (
    <ToolsListPage title={t("reflinks.title")}>
      <ReflinksTable promises={promises} workspaceId={useWorkspaceId()} />
    </ToolsListPage>
  )
}

export function QrCodesPage() {
  const t = useTranslations()
  const promises = useListPromise(listQrCodesSearchParamsCache, listQrCodes)
  return (
    <ToolsListPage fallback={<div>Loading...</div>} title={t("qrCodes.title")}>
      <QrCodesTable promises={promises} workspaceId={useWorkspaceId()} />
    </ToolsListPage>
  )
}

export function MinigamesPage() {
  const t = useTranslations()
  const promises = useListPromise(listMinigamesSearchParamsCache, listMinigames)
  return (
    <ToolsListPage
      fallback={<div>{t("actions.loading")}</div>}
      title={t("minigames.title")}
    >
      <MinigamesTable promises={promises} workspaceId={useWorkspaceId()} />
    </ToolsListPage>
  )
}

/** `pipelines/layout.tsx`: `requireContactsAccess`. */
export function PipelinesLayout() {
  const { permissions } = useWorkspacePage()
  if (!hasContactsAccess(permissions)) {
    return <NotFoundPage />
  }
  return <Outlet />
}

export function PipelinesPage() {
  const t = useTranslations()
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb
        items={[{ label: t("fields.pipelines.label"), href: "" }]}
      />
      <PipelinesList workspaceId={useWorkspaceId()} />
    </div>
  )
}
