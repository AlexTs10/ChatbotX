// Generated from apps/builder/src/features/ai-files/actions/reindex-ai-file.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const reindexAIFileAction = serverAction("reindexAIFileAction", {
  workspace: true,
})
