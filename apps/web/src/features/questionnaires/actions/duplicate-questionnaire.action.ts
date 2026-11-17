// Generated from apps/builder/src/features/questionnaires/actions/duplicate-questionnaire.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const duplicateQuestionnaireAction = serverAction(
  "duplicateQuestionnaireAction",
  { workspace: true, noInput: true },
)
