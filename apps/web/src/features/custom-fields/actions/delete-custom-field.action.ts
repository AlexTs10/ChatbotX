// Generated from apps/builder/src/features/custom-fields/actions/delete-custom-field.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const deleteFieldsAction = serverAction("deleteFieldsAction", {
  workspace: true,
})
