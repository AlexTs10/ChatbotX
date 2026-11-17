// apps/builder/src/features/integration-openai/queries for the SPA: the same
// read in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type { IntegrationOpenAIResource } from "../schema/request"

export const findIntegrationOpenAI = ({
  workspaceId,
}: {
  workspaceId: string
}): Promise<{ data: IntegrationOpenAIResource | null }> =>
  serverQuery("findIntegrationOpenAI", workspaceId, [{ workspaceId }])
