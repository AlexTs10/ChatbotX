// Generated from apps/builder/src/features/integration-telegram/actions/disconnect.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const disconnectTelegramAction = serverAction(
  "disconnectTelegramAction",
  { workspace: true, noInput: true },
)
