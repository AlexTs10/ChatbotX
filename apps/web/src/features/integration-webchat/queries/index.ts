// apps/builder/src/features/integration-webchat/queries for the SPA: the
// list in the Rust server (P4 spec §5.1).
import type { IntegrationWebchatModel } from "@chatbotx.io/database/types"
import { serverQuery } from "@/lib/server-action"
import type { ListIntegrationWebchatsRequest } from "../schema/query"

export const listIntegrationWebchats = (
  input: ListIntegrationWebchatsRequest & { workspaceId: string },
): Promise<{ data: IntegrationWebchatModel[]; pageCount: number }> =>
  serverQuery("listIntegrationWebchats", input.workspaceId, [input])
