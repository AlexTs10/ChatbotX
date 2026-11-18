// `inboxTeamService.listByWorkspace` (the inbox-teams settings page) through
// the ported `inboxTeamsAPI.listInboxTeamsAuthenticatedAPI` procedure.
import { client } from "@/lib/orpc/orpc"
import type {
  ListInboxTeamsRequest,
  ListInboxTeamsResponse,
} from "../schema/action"

export const listInboxTeams = (
  input: ListInboxTeamsRequest,
): Promise<ListInboxTeamsResponse> =>
  client.inboxTeamsAPI.listInboxTeamsAuthenticatedAPI({
    workspaceId: input.workspaceId,
  }) as Promise<ListInboxTeamsResponse>
