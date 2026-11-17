// apps/builder/src/features/ai-functions/queries for the SPA: the same list
// through the Rust `/rpc` (P4 spec §5).
import type { AIFunctionModel } from "@chatbotx.io/database/types"
import type { PaginatedResponse } from "@/features/common/schema/pagination"
import { client } from "@/lib/orpc/orpc"
import type { ListAIFunctionsRequest } from "../schema/action"

// ponytail: the procedure lists the first page without `pageCount`; the
// table shows one page, as the builder's default request did.
export const listAIFunctions = async (
  input: ListAIFunctionsRequest,
): Promise<PaginatedResponse<AIFunctionModel>> => ({
  pageCount: 1,
  ...(await client.aiFunctionsAPI.listAIFunctionsAuthenticatedAPI({
    workspaceId: input.workspaceId,
  })),
})
