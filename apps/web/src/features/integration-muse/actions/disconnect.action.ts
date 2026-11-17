// Generated from apps/builder/src/features/integration-muse/actions/disconnect.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const disconnectMuseAction = serverAction("disconnectMuseAction", {
  workspace: true,
  noInput: true,
})
