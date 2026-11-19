// apps/builder/src/features/integration-smtp/queries for the SPA: the list
// runs in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type { IntegrationSmtpResource } from "../schema/resource"

export const listIntegrationSmtps = ({
  workspaceId,
}: {
  workspaceId: string
}): Promise<{ data: IntegrationSmtpResource[] }> =>
  serverQuery("listIntegrationSmtps", workspaceId, [{ workspaceId }])
