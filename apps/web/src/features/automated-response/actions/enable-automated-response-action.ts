// Generated from apps/builder/src/features/automated-response/actions/enable-automated-response-action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction, serverFunction } from "@/lib/server-action"

export const enableAutomatedResponseAction = serverAction("enableAutomatedResponseAction", { workspace: true })
export const enableAutomatedResponse = serverFunction("enableAutomatedResponse")
