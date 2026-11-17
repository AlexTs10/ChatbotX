// page: app/space/[workspaceId]/appointment-calendars/layout.tsx
// page: app/space/[workspaceId]/appointment-calendars/page.tsx
// page: app/space/[workspaceId]/appointment-calendars/appointments/page.tsx
// page: app/space/[workspaceId]/appointment-calendars/external/page.tsx
// page: app/space/[workspaceId]/appointment-calendars/[id]/edit/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/inbox-teams/working-hours/[workingHoursId]/page.tsx
// Membership is the workspace layout's check; the Rust queries scope every
// row to the workspace (a foreign id is a 404).
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet, useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { Suspense, useMemo } from "react"
import { AppBreadcrumb } from "@/components/app-breadcrumb"
import { AppointmentCalendarsTab } from "@/features/appointment-calendars/components/appointment-calendars-tab"
import { AppointmentCalendarsTable } from "@/features/appointment-calendars/components/appointment-calendars-table"
import { EditAppointmentCalendarForm } from "@/features/appointment-calendars/components/edit-appointment-calendar-form"
import {
  getAppointmentCalendar,
  listAppointmentCalendars,
} from "@/features/appointment-calendars/queries"
import { listAppointmentCalendarsSearchParamsCache } from "@/features/appointment-calendars/schema/query"
import { AppointmentsTable } from "@/features/appointment-management/components/appointments-table"
import { listAppointments } from "@/features/appointment-management/queries"
import {
  listAppointmentsSearchParamsCache,
  normalizeAppointmentManagementTab,
} from "@/features/appointment-management/schema/query"
import { ExternalCalendarsTable } from "@/features/external-calendars/components/external-calendars-table"
import {
  listExternalCalendars,
  listExternalCalendarsForSelect,
} from "@/features/external-calendars/queries"
import { LockedEditorGuard } from "@/features/templates/components/locked-editor-guard"
import { WorkingHoursScheduleForm } from "@/features/working-hours/components/working-hours-schedule-form"
import { getWorkingHoursEditorPage } from "@/features/working-hours/queries"
import { workingHoursListHref } from "@/features/working-hours/lib/routes"
import { useListPromise, useWorkspaceId } from "@/routes/tools"

function useCalendarsCrumbs(last: string) {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  return [
    { label: t("tools.title"), href: `/space/${workspaceId}/tools` },
    {
      label: t("appointmentCalendars.title"),
      href: last ? `/space/${workspaceId}/appointment-calendars` : "",
    },
    ...(last ? [{ label: last, href: "" }] : []),
  ]
}

export function AppointmentCalendarsLayout() {
  return (
    <div className="flex flex-col gap-4">
      <AppointmentCalendarsTab />
      <Outlet />
    </div>
  )
}

export function AppointmentCalendarsPage() {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  const promises = useListPromise(
    listAppointmentCalendarsSearchParamsCache,
    listAppointmentCalendars,
  )
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb items={useCalendarsCrumbs("")} />
      <Suspense fallback={<div>{t("actions.loading")}</div>}>
        <AppointmentCalendarsTable
          promises={promises}
          workspaceId={workspaceId}
        />
      </Suspense>
    </div>
  )
}

export function AppointmentsPage() {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  const searchParams = useSearchParams()
  const tab = normalizeAppointmentManagementTab(searchParams.get("tab"))
  const promises = useListPromise(listAppointmentsSearchParamsCache, (search) =>
    listAppointments({ ...search, tab }),
  )
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb
        items={useCalendarsCrumbs(t("appointmentManagement.title"))}
      />
      <Suspense fallback={<div>{t("actions.loading")}</div>}>
        <AppointmentsTable
          promises={promises}
          tab={tab}
          workspaceId={workspaceId}
        />
      </Suspense>
    </div>
  )
}

export function ExternalCalendarsPage() {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  const promises = useMemo(
    () => Promise.all([listExternalCalendars({ workspaceId })]),
    [workspaceId],
  )
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb items={useCalendarsCrumbs(t("externalCalendars.title"))} />
      <Suspense fallback={<div>{t("actions.loading")}</div>}>
        <ExternalCalendarsTable promises={promises} workspaceId={workspaceId} />
      </Suspense>
    </div>
  )
}

export function AppointmentCalendarEditPage() {
  const { workspaceId, id } = useParams({ strict: false }) as {
    workspaceId: string
    id: string
  }
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "appointmentCalendarEdit", workspaceId, id],
    queryFn: () =>
      Promise.all([
        getAppointmentCalendar({ workspaceId, id }),
        listExternalCalendarsForSelect({ workspaceId }),
      ]),
  })
  const [calendar, externalCalendars] = data
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb items={useCalendarsCrumbs(calendar.name)} />
      <LockedEditorGuard
        resourceId={id}
        resourceKind="calendar"
        workspaceId={workspaceId}
      >
        <EditAppointmentCalendarForm
          calendar={calendar}
          externalCalendarOptions={externalCalendars.map((connection) => ({
            label: connection.label,
            value: connection.id,
          }))}
          workspaceId={workspaceId}
        />
      </LockedEditorGuard>
    </div>
  )
}

export function WorkingHoursEditorPage() {
  const t = useTranslations()
  const { workspaceId, workingHoursId } = useParams({ strict: false }) as {
    workspaceId: string
    workingHoursId: string
  }
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "workingHoursEditor", workspaceId, workingHoursId],
    queryFn: () =>
      getWorkingHoursEditorPage({ workspaceId, id: workingHoursId }),
  })
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb
        items={[
          {
            label: t("inboxTeams.title"),
            href: workingHoursListHref(workspaceId),
          },
          {
            label: t("workingHours.title"),
            href: workingHoursListHref(workspaceId),
          },
          { label: data.workingHours.name, href: "" },
        ]}
      />
      <Suspense>
        <WorkingHoursScheduleForm
          timezone={data.timezone}
          workingHours={data.workingHours}
          workspaceId={workspaceId}
        />
      </Suspense>
    </div>
  )
}
