// apps/builder/src/features/appointment-calendars/queries for the SPA: the
// Rust server runs it under its export name (P4 spec §5.1).
import type { appointmentCalendarService } from "@chatbotx.io/business"
import { serverQuery } from "@/lib/server-action"
import type { ListAppointmentCalendarsRequest } from "../schema/query"

export const listAppointmentCalendars = (
  input: ListAppointmentCalendarsRequest,
) =>
  serverQuery<Awaited<ReturnType<typeof appointmentCalendarService.list>>>(
    "listAppointmentCalendars",
    input.workspaceId,
    [input],
  )
