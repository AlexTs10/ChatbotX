// Generated from apps/builder/src/features/integration-openai/actions/update-openai.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction, serverFunction } from "@/lib/server-action"

export const updateIntegrationOpenAIAction = serverAction(
  "updateIntegrationOpenAIAction",
  { workspace: true },
)
export const updateIntegrationOpenAI = serverFunction("updateIntegrationOpenAI")
