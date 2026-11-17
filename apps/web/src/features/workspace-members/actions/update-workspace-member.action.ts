// Generated from apps/builder/src/features/workspace-members/actions/update-workspace-member.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const updateWorkspaceMemberAction = serverAction(
  "updateWorkspaceMemberAction",
  { workspace: true },
)
