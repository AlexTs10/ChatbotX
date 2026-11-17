// Generated from apps/builder/src/features/bot-fields/actions/reset-bot-field.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction, serverFunction } from "@/lib/server-action"

export const resetBotFields = serverFunction("resetBotFields")
export const resetBotFieldsAction = serverAction("resetBotFieldsAction", {
  workspace: true,
})
