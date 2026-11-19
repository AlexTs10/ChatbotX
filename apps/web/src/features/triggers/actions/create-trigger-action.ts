// Generated from apps/builder/src/features/triggers/actions/create-trigger-action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const createTriggerAction = serverAction("createTriggerAction", { workspace: true })
