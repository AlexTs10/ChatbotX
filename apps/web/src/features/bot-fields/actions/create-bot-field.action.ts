// Generated from apps/builder/src/features/bot-fields/actions/create-bot-field.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const createBotFieldAction = serverAction("createBotFieldAction", {
  workspace: true,
})
