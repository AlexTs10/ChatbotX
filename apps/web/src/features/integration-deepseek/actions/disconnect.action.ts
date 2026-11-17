// Generated from apps/builder/src/features/integration-deepseek/actions/disconnect.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const disconnectDeepSeekAction = serverAction(
  "disconnectDeepSeekAction",
  { workspace: true, noInput: true },
)
