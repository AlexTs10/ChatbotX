// Generated from apps/builder/src/features/import/actions/peek-import-headers.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const peekImportHeadersAction = serverAction("peekImportHeadersAction", {
  workspace: true,
})
