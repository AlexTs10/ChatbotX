// Generated from apps/builder/src/features/integration-threads/actions/reconnect.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const reconnectThreadsAction = serverAction("reconnectThreadsAction", { workspace: true, noInput: true })
