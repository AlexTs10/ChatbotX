// Generated from apps/builder/src/features/saved-replies/actions/delete-saved-reply.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const deleteSavedReplyAction = serverAction("deleteSavedReplyAction", {
  workspace: true,
  noInput: true,
})
