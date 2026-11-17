// page: app/space/[workspaceId]/questionnaires/[id]/edit/page.tsx
// page: app/space/[workspaceId]/questionnaires/[id]/applicants/page.tsx
// page: app/space/[workspaceId]/polls/create/page.tsx
// page: app/space/[workspaceId]/polls/[id]/edit/page.tsx
// page: app/space/[workspaceId]/polls/[id]/analytics/page.tsx
// page: app/space/[workspaceId]/qr-codes/create/page.tsx
// page: app/space/[workspaceId]/qr-codes/[id]/edit/page.tsx
// page: app/space/[workspaceId]/minigames/create/page.tsx
// page: app/space/[workspaceId]/minigames/[id]/edit/page.tsx
// page: app/space/[workspaceId]/minigames/[id]/history/page.tsx
// page: app/space/[workspaceId]/reflinks/[reflinkId]/analytics/page.tsx
// page: app/space/[workspaceId]/pipelines/[id]/page.tsx
// The tools detail pages. Membership is the workspace layout's check.
import { ORPCError } from "@orpc/client"
import { useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import { useTranslations } from "next-intl"
import { minigameTypes } from "@chatbotx.io/database/partials"
import { useSearchParams } from "next/navigation"
import { type ReactNode, Suspense, useMemo } from "react"
import { ReflinkAnalyticsClient } from "@/app/space/[workspaceId]/reflinks/[reflinkId]/analytics/reflink-analytics-client"
import { MinigameTab } from "@/app/space/[workspaceId]/minigames/[id]/tab"
import { AppBreadcrumb } from "@/components/app-breadcrumb"
import { MinigameHistoryTable } from "@/features/minigames/components/minigame-history-table"
import { buildMinigamePlayUrl } from "@/features/minigames/lib/play-url"
import { MinigameForm } from "@/features/minigames/minigame-form"
import { findMinigame, listMinigameHistory } from "@/features/minigames/queries"
import { listMinigameHistorySearchParamsCache } from "@/features/minigames/schema/query"
import type { MinigameResource } from "@/features/minigames/schema/resource"
import { PipelineBoard } from "@/features/pipelines/components/board/pipeline-board"
import { PollAnalytics } from "@/features/polls/components/poll-analytics"
import { PollForm } from "@/features/polls/components/poll-form"
import type { PollResource } from "@/features/polls/schema/resource"
import { CreateQrCodeForm } from "@/features/qr-codes/create-qr-code-form"
import { findQrCode } from "@/features/qr-codes/queries"
import { UpdateQrCodeForm } from "@/features/qr-codes/update-qr-code-form"
import { EditQuestionnaireForm } from "@/features/questionnaires/components/edit-questionnaire-form"
import { QuestionnaireApplicantsTable } from "@/features/questionnaires/components/questionnaire-applicants-table"
import { QuestionnaireDashboardSummary } from "@/features/questionnaires/components/questionnaire-dashboard-summary"
import { getQuestionnaire } from "@/features/questionnaires/queries/get-questionnaire.query"
import {
  getQuestionnaireDashboardSummary,
  listQuestionnaireSubmissions,
} from "@/features/questionnaires/queries/submissions.query"
import { listQuestionnaireSubmissionsSearchParamsCache } from "@/features/questionnaires/schema/query"
import { LockedIdsProvider } from "@/features/templates/components/lock-context"
import { LockedEditorGuard } from "@/features/templates/components/locked-editor-guard"
import { client } from "@/lib/orpc/orpc"
import { serverQuery } from "@/lib/server-action"
import { Redirect } from "@/shims/redirect"
import { NotFoundPage } from "@/routes/root"

function useIds() {
  return useParams({ strict: false }) as {
    workspaceId: string
    id?: string
    reflinkId?: string
  }
}

function ToolPage(props: {
  section: string
  sectionPath: string
  title: string
  children: ReactNode
}) {
  const t = useTranslations()
  const { workspaceId } = useIds()
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb
        items={[
          { label: t("tools.title"), href: `/space/${workspaceId}/tools` },
          {
            label: props.section,
            href: `/space/${workspaceId}/${props.sectionPath}`,
          },
          { label: props.title, href: "" },
        ]}
      />
      {props.children}
    </div>
  )
}

export function EditQuestionnairePage() {
  const t = useTranslations()
  const { workspaceId, id = "" } = useIds()
  const questionnaire = useSuspenseQuery({
    queryKey: ["spa", "getQuestionnaire", workspaceId, id],
    queryFn: () => getQuestionnaire({ workspaceId, id }),
  }).data
  return (
    <ToolPage
      section={t("questionnaires.title")}
      sectionPath="questionnaires"
      title={questionnaire.name}
    >
      <LockedEditorGuard
        resourceId={id}
        resourceKind="questionnaire"
        workspaceId={workspaceId}
      >
        <EditQuestionnaireForm
          questionnaire={questionnaire}
          workspaceId={workspaceId}
        />
      </LockedEditorGuard>
    </ToolPage>
  )
}

export function QuestionnaireApplicantsPage() {
  const t = useTranslations()
  const { workspaceId, id = "" } = useIds()
  const questionnaire = useSuspenseQuery({
    queryKey: ["spa", "getQuestionnaire", workspaceId, id],
    queryFn: () => getQuestionnaire({ workspaceId, id }),
  }).data
  const tablePromises = useSearchPromise(
    listQuestionnaireSubmissionsSearchParamsCache,
    (search) =>
      listQuestionnaireSubmissions({
        ...search,
        workspaceId,
        questionnaireId: id,
      }),
  )
  const summaryPromise = useMemo(
    () => getQuestionnaireDashboardSummary({ workspaceId, questionnaireId: id }),
    [workspaceId, id],
  )
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb
        items={[
          { label: t("tools.title"), href: `/space/${workspaceId}/tools` },
          {
            label: t("questionnaires.title"),
            href: `/space/${workspaceId}/questionnaires`,
          },
          {
            label: questionnaire.name,
            href: `/space/${workspaceId}/questionnaires/${id}/edit`,
          },
          { label: t("questionnaires.applicants"), href: "" },
        ]}
      />
      <Suspense fallback={<div>{t("actions.loading")}</div>}>
        <QuestionnaireDashboardSummary summary={summaryPromise} />
      </Suspense>
      <Suspense fallback={<div>{t("actions.loading")}</div>}>
        <QuestionnaireApplicantsTable
          promises={tablePromises}
          questionnaireId={id}
          workspaceId={workspaceId}
        />
      </Suspense>
    </div>
  )
}

/**
 * The list promise of a page, made once per search string (the table
 * suspends below, so the page keeps its memoized promise).
 */
function useSearchPromise<S, T>(
  cache: { all: () => S; parse: (input: Record<string, string>) => unknown },
  load: (search: S) => Promise<T>,
) {
  const searchParams = useSearchParams()
  const key = searchParams.toString()
  // biome-ignore lint/correctness/useExhaustiveDependencies: the search string is the key
  return useMemo(
    () =>
      Promise.all([
        load(cache.parse(Object.fromEntries(searchParams)) as S),
      ]) as Promise<[T]>,
    [key],
  )
}

/** `findPoll`: `undefined` (here `null`) only for "no such poll". */
function usePoll(): PollResource | null {
  const { workspaceId, id = "" } = useIds()
  return useSuspenseQuery({
    queryKey: ["spa", "findPoll", workspaceId, id],
    queryFn: async () => {
      try {
        return (await client.pollsAPI.getPollAPI({
          workspaceId,
          id,
        })) as PollResource
      } catch (error) {
        if (error instanceof ORPCError && error.status === 404) {
          return null
        }
        throw error
      }
    },
  }).data
}

export function CreatePollPage() {
  return <PollForm workspaceId={useIds().workspaceId} />
}

export function EditPollPage() {
  const poll = usePoll()
  const { workspaceId } = useIds()
  if (!poll) {
    return <NotFoundPage />
  }
  return <PollForm poll={poll} workspaceId={workspaceId} />
}

export function PollAnalyticsPage() {
  const t = useTranslations()
  const poll = usePoll()
  const { workspaceId, id = "" } = useIds()
  if (!poll) {
    return <NotFoundPage />
  }
  return (
    <ToolPage
      section={t("pollManager.title")}
      sectionPath="polls"
      title={t("pollManager.analytics.title")}
    >
      <PollAnalytics pollId={id} workspaceId={workspaceId} />
    </ToolPage>
  )
}

function ToolFormPage(props: {
  section: string
  sectionPath: string
  title: string
  children: ReactNode
}) {
  const t = useTranslations()
  const { workspaceId } = useIds()
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb
        items={[
          { label: t("tools.title"), href: `/space/${workspaceId}/tools` },
          {
            label: t(props.section),
            href: `/space/${workspaceId}/${props.sectionPath}`,
          },
          { label: props.title, href: "" },
        ]}
      />
      {props.children}
    </div>
  )
}

export function CreateQrCodePage() {
  const t = useTranslations()
  return (
    <ToolFormPage
      section="qrCodes.title"
      sectionPath="qr-codes"
      title={t("actions.create")}
    >
      <CreateQrCodeForm workspaceId={useIds().workspaceId} />
    </ToolFormPage>
  )
}

export function EditQrCodePage() {
  const t = useTranslations()
  const { workspaceId, id = "" } = useIds()
  const qrCode = useSuspenseQuery({
    queryKey: ["spa", "findQrCode", workspaceId, id],
    queryFn: () => findQrCode({ workspaceId, id }),
  }).data
  if (!qrCode) {
    return <NotFoundPage />
  }
  return (
    <ToolFormPage
      section="qrCodes.title"
      sectionPath="qr-codes"
      title={t("actions.edit")}
    >
      <LockedEditorGuard
        resourceId={id}
        resourceKind="reflink"
        workspaceId={workspaceId}
      >
        <UpdateQrCodeForm qrCode={qrCode} workspaceId={workspaceId} />
      </LockedEditorGuard>
    </ToolFormPage>
  )
}

export function CreateMinigamePage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const type = minigameTypes.safeParse(useSearchParams().get("type"))
  if (!type.success) {
    return <Redirect href={`/space/${workspaceId}/minigames`} />
  }
  return (
    <ToolFormPage
      section="minigames.title"
      sectionPath="minigames"
      title={t("actions.create")}
    >
      <MinigameForm mode="create" type={type.data} workspaceId={workspaceId} />
    </ToolFormPage>
  )
}

/** `minigames/[id]/layout.tsx`: the breadcrumb and the edit / history tabs. */
function MinigameFrame(props: {
  children: (minigame: MinigameResource) => ReactNode
}) {
  const t = useTranslations()
  const { workspaceId, id = "" } = useIds()
  const minigame = useSuspenseQuery({
    queryKey: ["spa", "findMinigame", workspaceId, id],
    queryFn: () => findMinigame({ workspaceId, id }),
  }).data
  if (!minigame) {
    return <NotFoundPage />
  }
  return (
    <ToolFormPage
      section="minigames.title"
      sectionPath="minigames"
      title={minigame.name}
    >
      <MinigameTab minigameId={id} />
      {props.children(minigame)}
    </ToolFormPage>
  )
}

export function EditMinigamePage() {
  const { workspaceId } = useIds()
  return (
    <MinigameFrame>
      {(minigame) => (
        <MinigameForm
          minigame={minigame}
          mode="edit"
          publicUrl={buildMinigamePlayUrl(minigame.id)}
          workspaceId={workspaceId}
        />
      )}
    </MinigameFrame>
  )
}

export function MinigameHistoryPage() {
  const t = useTranslations()
  const { workspaceId, id = "" } = useIds()
  const tablePromises = useSearchPromise(
    listMinigameHistorySearchParamsCache,
    (search) => listMinigameHistory({ ...search, workspaceId, minigameId: id }),
  )
  return (
    <MinigameFrame>
      {() => (
        <Suspense fallback={<div>{t("actions.loading")}</div>}>
          <MinigameHistoryTable
            minigameId={id}
            promises={tablePromises}
            workspaceId={workspaceId}
          />
        </Suspense>
      )}
    </MinigameFrame>
  )
}

export function ReflinkAnalyticsPage() {
  const { workspaceId, reflinkId = "" } = useIds()
  const reflink = useSuspenseQuery({
    queryKey: ["spa", "findReflink", workspaceId, reflinkId],
    queryFn: () =>
      serverQuery<{ name: string } | null>("findReflink", workspaceId, [
        { workspaceId, id: reflinkId },
      ]),
  }).data
  if (!reflink) {
    return <NotFoundPage />
  }
  return (
    <div className="container mx-auto flex flex-col gap-6 py-6">
      <ReflinkAnalyticsClient
        linkId={reflinkId}
        linkName={reflink.name}
        workspaceId={workspaceId}
      />
    </div>
  )
}

/** `canViewPipeline`: the pipeline query answers 404 when it is hidden. */
export function PipelineBoardPage() {
  const t = useTranslations()
  const { workspaceId, id: pipelineId = "" } = useIds()
  const canView = useSuspenseQuery({
    queryKey: ["spa", "canViewPipeline", workspaceId, pipelineId],
    queryFn: async () => {
      try {
        await client.pipelinesAPI.privateGetPipelineAPI({
          workspaceId,
          pipelineId,
        })
        return true
      } catch (error) {
        if (error instanceof ORPCError && error.status === 404) {
          return false
        }
        throw error
      }
    },
  }).data
  if (!canView) {
    return <NotFoundPage />
  }
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      <AppBreadcrumb
        items={[
          {
            label: t("fields.pipelines.label"),
            href: `/space/${workspaceId}/pipelines`,
          },
          { label: t("pipelines.board.title"), href: "" },
        ]}
      />
      <LockedIdsProvider
        resourceIds={[pipelineId]}
        resourceKind="pipeline"
        workspaceId={workspaceId}
      >
        <PipelineBoard pipelineId={pipelineId} workspaceId={workspaceId} />
      </LockedIdsProvider>
    </div>
  )
}
