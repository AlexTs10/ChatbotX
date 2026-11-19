// Generated from apps/builder/src/features/integration-threads/actions/disconnect.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const disconnectThreadsAction = serverAction("disconnectThreadsAction", { workspace: true, noInput: true })
