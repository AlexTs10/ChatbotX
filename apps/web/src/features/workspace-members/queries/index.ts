// apps/builder/src/features/workspace-members/queries for the SPA: the list
// through the Rust `/rpc` (the procedure runs the same query, P4 spec §5).
import { client } from "@/lib/orpc/orpc"
import type {
  ListWorkspaceMembersRequest,
  ListWorkspaceMembersResponse,
} from "../schema/query"

export const listWorkspaceMembers = (
  input: ListWorkspaceMembersRequest,
): Promise<ListWorkspaceMembersResponse> =>
  client.workspaceMembersAPI.listWorkspaceMembersAuthenticatedAPI(input)
