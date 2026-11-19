// apps/builder/src/features/integration-zalo/queries for the SPA: the list
// (no credentials) runs in the Rust server.
import { serverQuery } from "@/lib/server-action"

export type IntegrationZaloResource = {
  id: string
  inboxId: string
  name: string
  oaId: string
  syncTagEnabledAt: string | null
  tokenRefreshError: string | null
}

export const listIntegrationZalo = ({
  workspaceId,
}: {
  workspaceId: string
}): Promise<{ data: IntegrationZaloResource[] }> =>
  serverQuery("listIntegrationZalo", workspaceId, [])
