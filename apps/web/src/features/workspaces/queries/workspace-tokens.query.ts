// `workspaceApiTokenService.listTokens` + `toWorkspaceApiTokenDto` (the
// workspace-token settings page) in the Rust server.
import { serverQuery } from "@/lib/server-action"
import type { WorkspaceApiTokenDto } from "../schema/workspace-token-dto"

export const listWorkspaceTokens = (workspaceId: string) =>
  serverQuery<WorkspaceApiTokenDto[]>("listWorkspaceTokens", workspaceId, [
    { workspaceId },
  ])
