// Generated from apps/builder/src/features/automated-response/actions/update-automated-response-action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction, serverFunction } from "@/lib/server-action"

export const updateAutomatedResponseAction = serverAction("updateAutomatedResponseAction", { workspace: true })
export const updateAutomatedResponse = serverFunction("updateAutomatedResponse")
