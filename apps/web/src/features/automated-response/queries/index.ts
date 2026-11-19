// apps/builder/src/features/automated-response/queries for the SPA: the same
// reads through the Rust `/rpc` (`rpc_procedures/automated_responses.rs`).
import type { PaginatedResponse } from "@/features/common/schema/pagination"
import { serverQuery } from "@/lib/server-action"
import type {
  FindAutomatedResponseRequest,
  ListAutomatedResponsesRequest,
} from "../schema/query"
import type { AutomatedResponseResource } from "../schema/resource"

export const listAutomatedResponses = (
  input: ListAutomatedResponsesRequest,
): Promise<PaginatedResponse<AutomatedResponseResource>> =>
  serverQuery("listAutomatedResponses", input.workspaceId, [
    input.workspaceId,
    input,
  ])

/** `null` when the rule of that id and type does not exist. */
export const findAutomatedResponse = (
  input: FindAutomatedResponseRequest,
): Promise<AutomatedResponseResource | null> =>
  serverQuery("findAutomatedResponse", input.workspaceId, [
    input.workspaceId,
    input,
  ])
