// Generated from apps/builder/src/features/workspaces/actions/delete-workspace-token-action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const deleteWorkspaceTokenAction = serverAction("deleteWorkspaceTokenAction", { workspace: true })
