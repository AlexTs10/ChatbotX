// apps/builder/src/features/integration-openrouter/queries for the SPA: the same
// read in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type { IntegrationOpenrouterResource } from "../schema/resource"

export const findIntegrationOpenRouter = ({
  workspaceId,
}: {
  workspaceId: string
}): Promise<IntegrationOpenrouterResource | null> =>
  serverQuery("findIntegrationOpenRouter", workspaceId, [{ workspaceId }])
