// apps/builder/src/features/reflinks/queries for the SPA:
// `reflinkService.list` in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type { ListReflinksRequest, ListReflinksResponse } from "../schema/query"

export const listReflinks = (input: ListReflinksRequest) =>
  serverQuery<ListReflinksResponse>("listReflinks", input.workspaceId, [input])
