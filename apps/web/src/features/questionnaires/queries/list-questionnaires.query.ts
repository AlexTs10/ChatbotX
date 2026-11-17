// apps/builder/src/features/questionnaires/queries/list-questionnaires.query.ts
// for the SPA: `questionnaireService.list` in the Rust server.
import type { QuestionnaireListItem } from "@/features/questionnaires/schema/resource"
import { serverQuery } from "@/lib/server-action"
import type { ListQuestionnairesRequest } from "../schema/query"

export const listQuestionnaires = (input: ListQuestionnairesRequest) =>
  serverQuery<{ data: QuestionnaireListItem[]; pageCount: number }>(
    "listQuestionnaires",
    input.workspaceId,
    [input],
  )
