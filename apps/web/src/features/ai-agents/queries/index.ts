// apps/builder/src/features/ai-agents/queries for the SPA: the same list
// through the Rust `/rpc` (P4 spec §5).
import type { AIAgentModel } from "@chatbotx.io/database/types"
import type { PaginatedResponse } from "@/features/common/schema/pagination"
import { client } from "@/lib/orpc/orpc"
import type { ListAIAgentsRequest } from "../schema/query"

export const listAIAgents = (
  input: ListAIAgentsRequest,
): Promise<PaginatedResponse<AIAgentModel>> =>
  client.aiAgentsAPI.listAIAgentsAPI(input)
