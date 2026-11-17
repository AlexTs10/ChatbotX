// Generated from apps/builder/src/features/flows/actions/update-flow-action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const updateFlowAction = serverAction("updateFlowAction", {
  workspace: true,
})
