// Generated from apps/builder/src/features/ai-functions/actions/delete-ai-function.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const deleteAIFunctionAction = serverAction("deleteAIFunctionAction", {
  workspace: true,
  noInput: true,
})
