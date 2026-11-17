// Generated from apps/builder/src/features/messages/actions/change-message-attributes.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction, serverFunction } from "@/lib/server-action"

export const changeMessageAttributes = serverFunction("changeMessageAttributes")
export const changeMessageAttributesAction = serverAction(
  "changeMessageAttributesAction",
  { workspace: true },
)
