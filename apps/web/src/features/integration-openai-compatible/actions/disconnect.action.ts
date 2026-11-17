// Generated from apps/builder/src/features/integration-openai-compatible/actions/disconnect.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const disconnectOpenaiCompatibleAction = serverAction(
  "disconnectOpenaiCompatibleAction",
  { workspace: true, noInput: true },
)
