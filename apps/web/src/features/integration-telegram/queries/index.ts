// apps/builder/src/features/integration-telegram/queries for the SPA: the
// list in the Rust server (P4 spec §5.1); `auth` comes without the token.
import type { IntegrationTelegramModel } from "@chatbotx.io/database/types"
import { serverQuery } from "@/lib/server-action"

export const listIntegrationTelegrams = ({
  where,
}: {
  where: { workspaceId: string }
}): Promise<{ data: IntegrationTelegramModel[] }> =>
  serverQuery("listIntegrationTelegrams", where.workspaceId, [{ where }])
