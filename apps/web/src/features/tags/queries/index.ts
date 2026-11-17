// apps/builder/src/features/tags/queries for the SPA: the same read through
// the Rust `/rpc` (P4 spec §5).
import { client } from "@/lib/orpc/orpc"
import type { ListTagsRequest, ListTagsResponse } from "../schema/query"

export const listTags = (
  input: ListTagsRequest & { workspaceId: string },
): Promise<ListTagsResponse> =>
  client.tagsAPI.privateListWorkspaceTagsAPI(input)

export const listTagsRSC = listTags
