// Generated from apps/builder/src/features/conversations/actions/sync-thread-owner.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

// biome-ignore lint/suspicious/noExplicitAny: server type, not ported
export type SyncThreadOwnerActionResult = any
export const syncThreadOwnerAction = serverAction("syncThreadOwnerAction", {
  workspace: true,
})
