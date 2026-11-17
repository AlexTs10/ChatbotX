// Generated from apps/builder/src/features/integration-gemini/actions/disconnect.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const disconnectGeminiAction = serverAction("disconnectGeminiAction", {
  workspace: true,
  noInput: true,
})
