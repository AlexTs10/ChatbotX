// apps/builder/src/features/tiktok-comments/queries for the SPA: the public
// `tiktokComments.list` / `tiktokComments.get` routes in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type {
  ListTiktokCommentsRequest,
  ListTiktokCommentsResponse,
} from "../schema/action"
import type { TiktokCommentResource } from "../schema/resource"

export const listTiktokComments = (input: ListTiktokCommentsRequest) =>
  serverQuery<ListTiktokCommentsResponse>("listTiktokComments", input.workspaceId, [input])

export const getTiktokComment = (workspaceId: string, id: string) =>
  serverQuery<TiktokCommentResource>("getTiktokComment", workspaceId, [{ id }])
