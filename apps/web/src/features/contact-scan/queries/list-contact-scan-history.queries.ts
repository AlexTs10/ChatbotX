// apps/builder/src/features/contact-scan/queries/list-contact-scan-history.queries.ts
// for the SPA: the Rust `/rpc` page query (`rpc_procedures/contact_tools.rs`)
// also runs the unrestricted-contacts gate.
import { serverQuery } from "@/lib/server-action"
import type {
  ListContactScanHistoryRequest,
  ListContactScanHistoryResponse,
} from "../schema/query"

export const listContactScanHistory = (
  input: ListContactScanHistoryRequest & { workspaceId: string },
): Promise<ListContactScanHistoryResponse> =>
  serverQuery("listContactScanHistory", input.workspaceId, [
    input.workspaceId,
    input,
  ])
