// The audit-logs page queries of apps/builder/src/enterprise/features/audit-logs/queries
// in the Rust server (enterprise features + workspace super admin there).
import type { listAuditLogs as listAuditLogsService } from "@chatbotx.io/business/audit"
import { serverQuery } from "@/lib/server-action"
import type { ListAuditLogsRequest } from "../schema/query"

export type AuditLogAdminOption = {
  id: string
  label: string
}

export const listAuditLogs = (input: ListAuditLogsRequest) =>
  serverQuery<Awaited<ReturnType<typeof listAuditLogsService>>>(
    "listAuditLogs",
    input.workspaceId,
    [input],
  )

export const listAuditLogAdmins = (workspaceId: string) =>
  serverQuery<AuditLogAdminOption[]>("listAuditLogAdmins", workspaceId, [
    workspaceId,
  ])
