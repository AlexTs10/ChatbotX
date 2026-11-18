// apps/builder/src/features/external-calendars/queries for the SPA: the
// Rust server runs them under their export names (P4 spec §5.1).
import type { ExternalCalendarListItem } from "@chatbotx.io/business"
import { serverQuery } from "@/lib/server-action"

export const listExternalCalendars = (input: { workspaceId: string }) =>
  serverQuery<{
    data: ExternalCalendarListItem[]
    total: number
    pageCount: number
  }>("listExternalCalendars", input.workspaceId, [input])

export const listExternalCalendarsForSelect = (input: { workspaceId: string }) =>
  serverQuery<ExternalCalendarListItem[]>(
    "listExternalCalendarsForSelect",
    input.workspaceId,
    [input],
  )
