// apps/builder/src/features/integration-deepseek/queries for the SPA: the same
// read in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type { IntegrationDeepseekResource } from "../schema/resource"

export const findIntegrationDeepSeek = ({
  workspaceId,
}: {
  workspaceId: string
}): Promise<IntegrationDeepseekResource | null> =>
  serverQuery("findIntegrationDeepSeek", workspaceId, [{ workspaceId }])
