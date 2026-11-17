// apps/builder/src/features/flows/queries for the SPA: the same reads
// through the Rust `/rpc` (P4 spec §5).
import { client } from "@/lib/orpc/orpc"
import type { ListFlowsRequest, ListFlowsResponse } from "../schema/query"

export const listFlowsRSC = (
  input: ListFlowsRequest & { workspaceId: string },
): Promise<ListFlowsResponse> => client.flowsAPI.privateListFlowsAPI(input)

export const listFlows = listFlowsRSC
