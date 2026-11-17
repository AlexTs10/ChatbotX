// Generated from apps/builder/src/features/integration-openai/actions/disconnect.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const disconnectOpenAIAction = serverAction("disconnectOpenAIAction", {
  workspace: true,
  noInput: true,
})
