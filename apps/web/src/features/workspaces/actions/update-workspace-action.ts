// Generated from apps/builder/src/features/workspaces/actions/update-workspace-action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const updateWorkspaceBasicAction = serverAction(
  "updateWorkspaceBasicAction",
  { workspace: true },
)
export const updateWorkspaceAdvancedAction = serverAction(
  "updateWorkspaceAdvancedAction",
  { workspace: true },
)
export const updateSmartResponseDelayAction = serverAction(
  "updateSmartResponseDelayAction",
  { workspace: true },
)
