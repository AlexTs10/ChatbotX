// apps/builder/src/features/integration-api/queries for the SPA: the list
// in the Rust server (P4 spec §5.1). The internal lookups are server code.
import { serverQuery } from "@/lib/server-action"
import type { ApiResource } from "../schema/resource"

export const listIntegrationApis = ({
  workspaceId,
}: {
  workspaceId: string
}): Promise<{ data: ApiResource[] }> =>
  serverQuery("listIntegrationApis", workspaceId, [{ workspaceId }])
