// The data of apps/builder settings/inbox-teams/working-hours/[workingHoursId]/page.tsx:
// `workingHoursEditorPage` in the Rust server (`workingHours.get` + the
// workspace timezone; 404 when the schedule is not in the workspace).
import { serverQuery } from "@/lib/server-action"
import type {
  WorkingHoursHolidayResource,
  WorkingHoursResource,
} from "../schema/resource"

export const getWorkingHoursEditorPage = (input: {
  workspaceId: string
  id: string
}) =>
  serverQuery<{ workingHours: WorkingHoursResource; timezone: string }>(
    "workingHoursEditorPage",
    input.workspaceId,
    [{ id: input.id }],
  )

/** The working-hours cards of `settings/inbox-teams/page.tsx`. */
export const getWorkingHoursListPage = (input: { workspaceId: string }) =>
  serverQuery<{
    workingHours: WorkingHoursResource[]
    members: { value: string; label: string }[]
    holidays: WorkingHoursHolidayResource[]
  }>("workingHoursListPage", input.workspaceId, [])
