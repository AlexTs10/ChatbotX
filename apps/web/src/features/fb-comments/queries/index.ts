// apps/builder/src/features/fb-comments/queries for the SPA: the public
// `fbComments.list` / `fbComments.get` routes in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type {
  ListFbCommentsRequest,
  ListFbCommentsResponse,
} from "../schema/action"
import type { FBCommentResource } from "../schema/resource"

export const listFbComments = (input: ListFbCommentsRequest) =>
  serverQuery<ListFbCommentsResponse>("listFbComments", input.workspaceId, [input])

export const getFbComment = (workspaceId: string, id: string) =>
  serverQuery<FBCommentResource>("getFbComment", workspaceId, [{ id }])
