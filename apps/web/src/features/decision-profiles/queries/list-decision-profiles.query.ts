// apps/builder/src/features/decision-profiles/queries for the SPA:
// `decisionProfileService.listForSettings` in the Rust server.
import { serverQuery } from "@/lib/server-action"
import type { ListDecisionProfilesRequest } from "../schema/query"
import type { DecisionProfileListItem } from "../schema/resource"

export const listDecisionProfiles = (input: ListDecisionProfilesRequest) =>
  serverQuery<{ data: DecisionProfileListItem[]; pageCount: number }>(
    "listDecisionProfiles",
    input.workspaceId,
    [{ ...input, name: input.name ?? undefined }],
  )
