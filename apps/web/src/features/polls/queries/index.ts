// apps/builder/src/features/polls/queries for the SPA:
// `pollOperations.list` in the Rust server (P4 spec §5.1).
import type { PollListItem } from "@/features/polls/schema/resource"
import { serverQuery } from "@/lib/server-action"

export const listPolls = (input: {
  workspaceId: string
  page: number
  perPage: number
  name: string | null
}) =>
  serverQuery<{ data: PollListItem[]; pageCount: number }>(
    "listPolls",
    input.workspaceId,
    [input],
  )
