// apps/builder/src/features/integration-openai-compatible/queries for the
// SPA: the same read in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type { IntegrationOpenaiCompatibleResource } from "../schema/resource"

export const listIntegrationOpenaiCompatible = ({
  workspaceId,
}: {
  workspaceId: string
}): Promise<IntegrationOpenaiCompatibleResource[]> =>
  serverQuery("listIntegrationOpenaiCompatible", workspaceId, [{ workspaceId }])
