// Generated from apps/builder/src/features/ai-mcp-servers/actions/delete-ai-mcp-server.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction, serverFunction } from "@/lib/server-action"

export const deleteAIMcpServerAction = serverAction("deleteAIMcpServerAction", {
  workspace: true,
  noInput: true,
})
export const deleteAIMcpServer = serverFunction("deleteAIMcpServer")
