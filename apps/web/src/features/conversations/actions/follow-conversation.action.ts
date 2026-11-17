// Generated from apps/builder/src/features/conversations/actions/follow-conversation.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const followConversationAction = serverAction(
  "followConversationAction",
  { workspace: true, noInput: true },
)
