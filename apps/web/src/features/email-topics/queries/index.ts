// apps/builder/src/features/email-topics/queries for the SPA: `emailTopicService.list`
// through the same Rust read as `emailTopicsAPI.privateListWorkspaceEmailTopicsAPI`.
import { client } from "@/lib/orpc/orpc"
import type { ListEmailTopicsRequest } from "../schema/query"

export const listEmailTopicsRSC = (
  input: ListEmailTopicsRequest & { workspaceId: string },
) =>
  client.emailTopicsAPI.privateListWorkspaceEmailTopicsAPI(
    input as Parameters<
      typeof client.emailTopicsAPI.privateListWorkspaceEmailTopicsAPI
    >[0],
  )
