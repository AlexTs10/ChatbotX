// apps/builder/src/features/broadcasts/queries for the SPA: the same reads
// through the Rust `/rpc` (P4 spec §5.1, `rpc_procedures/broadcasts.rs`).
import type { PaginatedResponse } from "@/features/common/schema/pagination"
import { serverQuery } from "@/lib/server-action"
import type { GetBroadcastsSchema } from "../schema/query"
import type { BroadcastResourceWithRelations } from "../schema/resource"

/** One-off broadcasts and repeating series, never the runs of a series. */
export const listBroadcasts = (
  input: GetBroadcastsSchema,
): Promise<PaginatedResponse<BroadcastResourceWithRelations>> =>
  serverQuery("listBroadcasts", input.workspaceId, [input.workspaceId, input])
