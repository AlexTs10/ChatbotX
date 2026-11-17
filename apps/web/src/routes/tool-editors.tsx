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
import type { ReactNode } from "react"
import { AppBreadcrumb } from "@/components/app-breadcrumb"
import { PipelineBoard } from "@/features/pipelines/components/board/pipeline-board"
import { PollAnalytics } from "@/features/polls/components/poll-analytics"
import { PollForm } from "@/features/polls/components/poll-form"
import type { PollResource } from "@/features/polls/schema/resource"
import { EditQuestionnaireForm } from "@/features/questionnaires/components/edit-questionnaire-form"
import { getQuestionnaire } from "@/features/questionnaires/queries/get-questionnaire.query"
import { LockedIdsProvider } from "@/features/templates/components/lock-context"
import { LockedEditorGuard } from "@/features/templates/components/locked-editor-guard"
import { client } from "@/lib/orpc/orpc"
import { NotAvailablePage, NotFoundPage } from "@/routes/root"

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
  return (
    <ToolPage
      section={t("questionnaires.title")}
      sectionPath="questionnaires"
      title={t("questionnaires.title")}
    >
      <NotAvailablePage
        detail="The applicant list is not available yet in this version (questionnaire submissions are not in the Rust server pages yet)."
        title={t("questionnaires.title")}
      />
    </ToolPage>
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

/** A tools page whose builder actions do not run in the Rust server yet. */
function notAvailable(sectionKey: string, sectionPath: string, what: string) {
  return function NotAvailableToolPage() {
    const t = useTranslations()
    return (
      <ToolPage
        section={t(sectionKey)}
        sectionPath={sectionPath}
        title={t(sectionKey)}
      >
        <NotAvailablePage
          detail={`${what} is not available yet in this version.`}
          title={t(sectionKey)}
        />
      </ToolPage>
    )
  }
}

export const CreateQrCodePage = notAvailable(
  "qrCodes.title",
  "qr-codes",
  "Creating a QR code",
)
export const EditQrCodePage = notAvailable(
  "qrCodes.title",
  "qr-codes",
  "Editing a QR code",
)
export const CreateMinigamePage = notAvailable(
  "minigames.title",
  "minigames",
  "Creating a minigame",
)
export const EditMinigamePage = notAvailable(
  "minigames.title",
  "minigames",
  "Editing a minigame",
)
export const MinigameHistoryPage = notAvailable(
  "minigames.title",
  "minigames",
  "The minigame play history",
)
export const ReflinkAnalyticsPage = notAvailable(
  "reflinks.title",
  "reflinks",
  "Reflink analytics",
)

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
