// apps/builder/src/features/integration-muse/queries for the SPA: the same
// read in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type { IntegrationMuseResource } from "../schema/resource"

export const findIntegrationMuse = ({
  workspaceId,
}: {
  workspaceId: string
}): Promise<IntegrationMuseResource | null> =>
  serverQuery("findIntegrationMuse", workspaceId, [{ workspaceId }])
