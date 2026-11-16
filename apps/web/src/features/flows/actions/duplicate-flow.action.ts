// Generated from apps/builder/src/features/flows/actions/duplicate-flow.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const duplicateFlowAction = serverAction("duplicateFlowAction", {
  workspace: true,
  noInput: true,
})
