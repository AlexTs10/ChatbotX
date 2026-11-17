// Generated from apps/builder/src/features/reflinks/actions/delete-reflinks.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const deleteReflinksAction = serverAction("deleteReflinksAction", {
  workspace: true,
})
