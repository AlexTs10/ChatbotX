// Generated from apps/builder/src/features/tags/actions/delete-tag-action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const deleteTagAction = serverAction("deleteTagAction", {
  workspace: true,
})
