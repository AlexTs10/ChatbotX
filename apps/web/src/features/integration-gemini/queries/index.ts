// apps/builder/src/features/integration-gemini/queries for the SPA: the same
// read in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type { IntegrationGeminiResource } from "../schema/resource"

export const findIntegrationGemini = ({
  workspaceId,
}: {
  workspaceId: string
}): Promise<IntegrationGeminiResource | null> =>
  serverQuery("findIntegrationGemini", workspaceId, [{ workspaceId }])
