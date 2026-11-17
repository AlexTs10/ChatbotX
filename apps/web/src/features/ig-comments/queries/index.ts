// apps/builder/src/features/ig-comments/queries for the SPA: the public
// `igComments.list` / `igComments.get` routes in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type {
  ListIgCommentsRequest,
  ListIgCommentsResponse,
} from "../schema/action"
import type { IgCommentResource } from "../schema/resource"

export const listIgComments = (input: ListIgCommentsRequest) =>
  serverQuery<ListIgCommentsResponse>("listIgComments", input.workspaceId, [input])

export const getIgComment = (workspaceId: string, id: string) =>
  serverQuery<IgCommentResource>("getIgComment", workspaceId, [{ id }])
