// apps/builder/src/features/ai-mcp-servers/queries for the SPA: the same
// list through the Rust `/rpc` (P4 spec §5).
import { client } from "@/lib/orpc/orpc"
import type {
  ListAIMcpServersRequest,
  ListAIMcpServersResponse,
} from "../schema/action"

export const listAIMcpServers = async (
  input: ListAIMcpServersRequest,
): Promise<ListAIMcpServersResponse & { pageCount: number }> => ({
  pageCount: 1,
  ...(await client.aiMcpServerAPIs.listAIMcpServersAuthenticatedAPI({
    workspaceId: input.workspaceId,
  })),
})
