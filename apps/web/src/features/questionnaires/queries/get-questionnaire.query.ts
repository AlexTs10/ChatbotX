// apps/builder/src/features/questionnaires/queries/get-questionnaire.query.ts
// for the SPA: `questionnaireService.getForEdit` in the Rust server (the
// public `questionnaires.get` resource; dates come back as `Date`).
import type { z } from "zod"
import { serverQuery } from "@/lib/server-action"
import type { questionnairePublicResource } from "../schema/public"

export const getQuestionnaire = (input: { workspaceId: string; id: string }) =>
  serverQuery<z.infer<typeof questionnairePublicResource>>(
    "getQuestionnaire",
    input.workspaceId,
    [input],
  )
