// Generated from apps/builder/src/features/workspace-members/actions/invite-workspace-member.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const inviteWorkspaceMemberAction = serverAction(
  "inviteWorkspaceMemberAction",
  { workspace: true },
)
