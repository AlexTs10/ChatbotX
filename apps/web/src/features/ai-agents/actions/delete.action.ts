// Generated from apps/builder/src/features/ai-agents/actions/delete.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const deleteAIAgentAction = serverAction("deleteAIAgentAction", {
  workspace: true,
})
