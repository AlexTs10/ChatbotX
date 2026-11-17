// Generated from apps/builder/src/features/messages/actions/delete-message.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction, serverFunction } from "@/lib/server-action"

export const deleteMessage = serverFunction("deleteMessage")
export const deleteMessageAction = serverAction("deleteMessageAction", {
  workspace: true,
})
