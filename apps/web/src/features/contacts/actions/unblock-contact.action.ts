// Generated from apps/builder/src/features/contacts/actions/unblock-contact.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const unblockContactAction = serverAction("unblockContactAction", {
  workspace: true,
  noInput: true,
})
