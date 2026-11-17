// Generated from apps/builder/src/features/workspaces/actions/schedule-workspace-deletion-action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const scheduleWorkspaceDeletionAction = serverAction(
  "scheduleWorkspaceDeletionAction",
  { workspace: true, noInput: true },
)
