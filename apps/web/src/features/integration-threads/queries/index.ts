// apps/builder/src/features/integration-threads/queries for the SPA: the list
// (no credentials) runs in the Rust server.
import { serverQuery } from "@/lib/server-action"

export type IntegrationThreadsResource = {
  id: string
  inboxId: string
  name: string
  threadsUserId: string
  username: string
  tokenRefreshError: string | null
}

export const listIntegrationThreads = ({
  workspaceId,
}: {
  workspaceId: string
}): Promise<{ data: IntegrationThreadsResource[] }> =>
  serverQuery("listIntegrationThreads", workspaceId, [])
