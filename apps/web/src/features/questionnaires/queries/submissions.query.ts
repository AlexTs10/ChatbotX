// The applicants page queries of apps/builder/src/features/questionnaires/queries
// (`questionnaireSubmissionService.list` / `.dashboard`) in the Rust server.
import { serverQuery } from "@/lib/server-action"
import type { ListQuestionnaireSubmissionsRequest } from "../schema/query"

export const listQuestionnaireSubmissions = (
  input: ListQuestionnaireSubmissionsRequest,
) =>
  serverQuery<
    Awaited<
      ReturnType<
        typeof import("@chatbotx.io/business").questionnaireSubmissionService.list
      >
    >
  >("listQuestionnaireSubmissions", input.workspaceId, [input])

export const getQuestionnaireDashboardSummary = (input: {
  workspaceId: string
  questionnaireId: string
}) =>
  serverQuery<{
    totalApplicants: number
    completed: number
    completionRate: number
  }>("getQuestionnaireDashboardSummary", input.workspaceId, [input])
