// Generated from apps/builder/src/features/integration-webchat/actions/delete-webchat.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const deleteWebchatAction = serverAction("deleteWebchatAction", {
  workspace: true,
  noInput: true,
})
