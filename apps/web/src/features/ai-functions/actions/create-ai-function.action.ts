// Generated from apps/builder/src/features/ai-functions/actions/create-ai-function.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const createAIFunctionAction = serverAction("createAIFunctionAction", {
  workspace: true,
})
