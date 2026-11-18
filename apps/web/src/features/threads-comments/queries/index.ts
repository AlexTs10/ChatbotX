// apps/builder/src/features/threads-comments/queries for the SPA: the public
// `threadsComments.list` / `threadsComments.get` routes in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type {
  ListThreadsCommentsRequest,
  ListThreadsCommentsResponse,
} from "../schema/action"
import type { ThreadsCommentResource } from "../schema/resource"

export const listThreadsComments = (input: ListThreadsCommentsRequest) =>
  serverQuery<ListThreadsCommentsResponse>("listThreadsComments", input.workspaceId, [input])

export const getThreadsComment = (workspaceId: string, id: string) =>
  serverQuery<ThreadsCommentResource>("getThreadsComment", workspaceId, [{ id }])
