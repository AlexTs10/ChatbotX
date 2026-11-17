// Generated from apps/builder/src/features/conversations/actions/unfollow-conversation.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const unfollowConversationAction = serverAction(
  "unfollowConversationAction",
  { workspace: true, noInput: true },
)
