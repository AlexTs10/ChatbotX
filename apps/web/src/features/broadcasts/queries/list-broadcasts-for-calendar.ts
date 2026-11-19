// apps/builder/src/features/broadcasts/queries/list-broadcasts-for-calendar.ts
// for the SPA: the window is computed here (the builder's pure date code), the
// rows come from the Rust `/rpc` (`listBroadcastsForCalendar`).
import type { BroadcastCalendarRow } from "@chatbotx.io/business"
import type { BroadcastStatus } from "@chatbotx.io/database/partials"
import { serverQuery } from "@/lib/server-action"
import type { BroadcastKind } from "../lib/broadcast-status"
import {
  type CalendarRange,
  getCalendarQueryRange,
  parseDateParam,
  parseEndDateParam,
} from "../lib/calendar-grid"

export function listBroadcastsForCalendar(input: {
  workspaceId: string
  range: CalendarRange
  date: string
  endDate: string
  status: BroadcastStatus | null
  kind?: BroadcastKind | null
  name: string | null
  timezone: string
}): Promise<BroadcastCalendarRow[]> {
  const anchor = parseDateParam(input.date)
  const endAnchor = parseEndDateParam(input.endDate, anchor)
  const { from, to } = getCalendarQueryRange(
    input.range,
    anchor,
    endAnchor,
    input.timezone,
  )
  return serverQuery("listBroadcastsForCalendar", input.workspaceId, [
    input.workspaceId,
    {
      from: from.toISOString(),
      to: to.toISOString(),
      status: input.status,
      kind: input.kind ?? null,
      name: input.name,
    },
  ])
}
