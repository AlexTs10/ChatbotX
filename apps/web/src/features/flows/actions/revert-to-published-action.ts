// Generated from apps/builder/src/features/flows/actions/revert-to-published-action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const revertToPublishedAction = serverAction("revertToPublishedAction", {
  workspace: true,
  noInput: true,
})
