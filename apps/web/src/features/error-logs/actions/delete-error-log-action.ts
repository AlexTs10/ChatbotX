// Generated from apps/builder/src/features/error-logs/actions/delete-error-log-action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const deleteErrorLogAction = serverAction("deleteErrorLogAction", { workspace: true })
