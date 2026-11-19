// apps/builder/src/features/triggers/queries for the SPA: the same reads
// through the Rust `/rpc` (`rpc_procedures/triggers.rs`).
import type { StoreSource } from "@chatbotx.io/database/partials"
import type UpdateTriggerForm from "../update-trigger-form"
import { serverQuery } from "@/lib/server-action"
import type { GetTriggersSchema, ListTriggersResponse } from "../schema/query"

export const getTriggers = (
  input: GetTriggersSchema,
): Promise<ListTriggersResponse> =>
  serverQuery("getTriggers", input.workspaceId, [input.workspaceId, input])

/** The edit page data: the trigger (`null` = not found) and the store source. */
export const findTriggerPage = (input: {
  workspaceId: string
  id: string
}): Promise<{
  trigger: Parameters<typeof UpdateTriggerForm>[0]["trigger"]
  storeSource: StoreSource
} | null> =>
  serverQuery("triggerEditPage", input.workspaceId, [input.workspaceId, input])
