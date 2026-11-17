// Generated from apps/builder/src/features/sequences/actions/delete-sequence.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const deleteSequenceAction = serverAction("deleteSequenceAction", {
  workspace: true,
  noInput: true,
})
