// apps/builder/src/features/integration-claude/queries for the SPA: the same
// read in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type { IntegrationClaudeResource } from "../schema/resource"

export const findIntegrationClaude = ({
  workspaceId,
}: {
  workspaceId: string
}): Promise<IntegrationClaudeResource | null> =>
  serverQuery("findIntegrationClaude", workspaceId, [{ workspaceId }])
