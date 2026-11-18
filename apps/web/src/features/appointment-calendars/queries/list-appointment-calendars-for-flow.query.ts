// apps/builder/src/features/appointment-calendars/queries for the SPA: the
// Rust server runs it under its export name (P4 spec §5.1).
import type { appointmentCalendarService } from "@chatbotx.io/business"
import { serverQuery } from "@/lib/server-action"

export const listAppointmentCalendarsForFlow = (input: {
  workspaceId: string
  keyword?: string
}) =>
  serverQuery<
    Awaited<ReturnType<typeof appointmentCalendarService.listForFlow>>
  >("listAppointmentCalendarsForFlow", input.workspaceId, [input])
