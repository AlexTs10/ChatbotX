// apps/builder/src/features/appointment-management/queries for the SPA: the
// Rust server runs it under its export name (P4 spec §5.1).
import type { appointmentService } from "@chatbotx.io/business"
import { serverQuery } from "@/lib/server-action"
import type { ListAppointmentsRequest } from "../schema/query"

export const listAppointments = (input: ListAppointmentsRequest) =>
  serverQuery<Awaited<ReturnType<typeof appointmentService.list>>>(
    "listAppointments",
    input.workspaceId,
    [input],
  )
