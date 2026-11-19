// apps/builder/src/features/integration-tiktok/queries for the SPA: the list
// (`toResource`, no credentials) runs in the Rust server.
import { serverQuery } from "@/lib/server-action"
import type { IntegrationTiktokResource } from "../schema/resource"

export const listIntegrationTiktoks = ({
  workspaceId,
}: {
  workspaceId: string
}): Promise<{ data: IntegrationTiktokResource[] }> =>
  serverQuery("listIntegrationTiktoks", workspaceId, [])
