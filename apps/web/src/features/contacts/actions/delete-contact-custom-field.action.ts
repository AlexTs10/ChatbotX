// Generated from apps/builder/src/features/contacts/actions/delete-contact-custom-field.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction, serverFunction } from "@/lib/server-action"

export const deleteContactCustomFieldAction = serverAction(
  "deleteContactCustomFieldAction",
  { workspace: true },
)
export const deleteContactCustomFields = serverFunction(
  "deleteContactCustomFields",
)
