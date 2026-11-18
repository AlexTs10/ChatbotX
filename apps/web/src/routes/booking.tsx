// page: app/booking/[slug]/page.tsx
// page: app/booking/picker/page.tsx
// page: app/booking/range-picker/page.tsx
// page: app/booking/schedule/page.tsx
// page: app/booking/cancel/page.tsx
// The public booking webview (lib/public-routes.ts: no session). The signed
// token is the authority: the Rust loaders `pages.booking*` verify it and
// answer `null` for a bad or expired link.
import { Badge } from "@chatbotx.io/ui/components/ui/badge"
import { useSuspenseQuery } from "@tanstack/react-query"
import { useSearchParams } from "next/navigation"
import Script from "next/script"
import { useTranslations } from "next-intl"
import type { ReactNode } from "react"
import { CancelAppointmentPanel } from "@/features/booking-webview/components/cancel-appointment-panel"
import { DateRangePicker } from "@/features/booking-webview/components/date-range-picker"
import { DateTimePicker } from "@/features/booking-webview/components/date-time-picker"
import { loadPage } from "@/lib/server-action"

type Translate = ReturnType<typeof useTranslations>

type ScheduleDetail = {
  id: string
  calendarName: string
  contactName: string | null
  startAt: string
  inviteeTimezone: string
  status: "scheduled" | "cancelled"
  locationType: string
  locationDetail: string | null
  confirmationMessage: string | null
  cancellable: boolean
  cancelToken?: string
}

function useToken() {
  return useSearchParams().get("token") ?? ""
}

/** The page data of a token, `null` for an invalid link. */
function useBookingPage<T>(name: string, token: string) {
  return useSuspenseQuery({
    queryKey: ["spa", `pages.${name}`, token],
    queryFn: () =>
      token ? loadPage<T | null>(name, { token }) : Promise.resolve(null),
  }).data
}

function PublicMessage({
  title,
  description,
}: {
  title: string
  description: string
}) {
  return (
    <div className="flex min-h-screen items-center justify-center p-6 text-center">
      <div className="max-w-md space-y-2">
        <h1 className="font-semibold text-2xl">{title}</h1>
        <p className="text-muted-foreground text-sm">{description}</p>
      </div>
    </div>
  )
}

function InvalidLink() {
  const t = useTranslations("bookingWebview")
  return (
    <PublicMessage
      description={t("errors.invalidTokenDescription")}
      title={t("errors.invalidTokenTitle")}
    />
  )
}

function MessengerExtensions({ enabled }: { enabled: boolean }) {
  return enabled ? (
    <Script
      src="https://connect.facebook.net/en_US/messenger.Extensions.js"
      strategy="afterInteractive"
    />
  ) : null
}

export function BookingSlugPage() {
  const t = useTranslations("bookingPage")
  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-2 p-6 text-center">
      <h1 className="font-semibold text-xl">{t("comingSoonTitle")}</h1>
      <p className="text-muted-foreground">{t("comingSoonDescription")}</p>
    </div>
  )
}

export function BookingPickerPage() {
  const token = useToken()
  const data = useBookingPage<{
    calendarName: string
    description: string | null
    timezone: string
    closeOnSuccess: boolean
    submitMode: "book" | "select"
    slots: { startAt: string; endAt: string }[]
  }>("bookingPicker", token)
  if (!data) {
    return <InvalidLink />
  }
  return (
    <>
      <MessengerExtensions enabled={data.closeOnSuccess} />
      <DateTimePicker
        calendarName={data.calendarName}
        closeOnSuccess={data.closeOnSuccess}
        description={data.description}
        slots={data.slots}
        submitMode={data.submitMode}
        timezone={data.timezone}
        token={token}
      />
    </>
  )
}

export function BookingRangePickerPage() {
  const token = useToken()
  const data = useBookingPage<{
    calendarName: string
    description: string | null
    timezone: string
    closeOnSuccess: boolean
  }>("bookingRangePicker", token)
  if (!data) {
    return <InvalidLink />
  }
  return (
    <>
      <MessengerExtensions enabled={data.closeOnSuccess} />
      <DateRangePicker
        calendarName={data.calendarName}
        closeOnSuccess={data.closeOnSuccess}
        description={data.description}
        timezone={data.timezone}
        token={token}
      />
    </>
  )
}

const statusLabelKeyByStatus = {
  scheduled: "schedule.status.scheduled",
  cancelled: "schedule.status.cancelled",
} as const

function DetailItem({
  label,
  children,
}: {
  label: string
  children: ReactNode
}) {
  return (
    <div className="space-y-1">
      <dt className="text-muted-foreground text-sm">{label}</dt>
      <dd className="font-medium text-sm">{children}</dd>
    </div>
  )
}

function formatDateTime(
  value: string,
  timezone: string,
  dateStyle: "full" | "medium",
) {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle,
    timeStyle: "short",
    timeZone: timezone,
  }).format(new Date(value))
}

function formatLocation(
  locationType: string,
  locationDetail: string | null,
  t: Translate,
) {
  if (locationDetail) {
    return locationDetail
  }
  if (locationType === "phoneCall") {
    return t("schedule.locationTypes.phoneCall")
  }
  if (locationType === "onlineMeeting") {
    return t("schedule.locationTypes.onlineMeeting")
  }
  return t("schedule.locationTypes.inPerson")
}

function CancelUnavailable() {
  const t = useTranslations("bookingWebview")
  return (
    <div className="rounded-md border bg-muted/40 p-4 text-sm">
      <p className="font-medium">{t("cancel.unavailableTitle")}</p>
      <p className="text-muted-foreground">
        {t("cancel.unavailableDescription")}
      </p>
    </div>
  )
}

export function BookingSchedulePage() {
  const t = useTranslations("bookingWebview")
  const detail = useBookingPage<ScheduleDetail>(
    "bookingSchedule",
    useToken(),
  )
  if (!detail) {
    return <InvalidLink />
  }
  return (
    <main className="mx-auto flex min-h-screen w-full max-w-2xl flex-col justify-center gap-6 p-6">
      <div className="space-y-3">
        <Badge variant={detail.status === "scheduled" ? "default" : "outline"}>
          {t(statusLabelKeyByStatus[detail.status])}
        </Badge>
        <div className="space-y-2">
          <h1 className="font-semibold text-3xl tracking-normal">
            {detail.calendarName}
          </h1>
          <p className="text-muted-foreground">
            {detail.confirmationMessage ?? t("schedule.defaultMessage")}
          </p>
        </div>
      </div>
      <dl className="grid gap-4 rounded-md border p-4 sm:grid-cols-2">
        <DetailItem label={t("schedule.fields.invitee")}>
          {detail.contactName ?? t("schedule.unknownInvitee")}
        </DetailItem>
        <DetailItem label={t("schedule.fields.time")}>
          {formatDateTime(detail.startAt, detail.inviteeTimezone, "full")}
        </DetailItem>
        <DetailItem label={t("schedule.fields.timezone")}>
          {detail.inviteeTimezone}
        </DetailItem>
        <DetailItem label={t("schedule.fields.location")}>
          {formatLocation(detail.locationType, detail.locationDetail, t)}
        </DetailItem>
      </dl>
      {detail.cancellable && detail.cancelToken ? (
        <CancelAppointmentPanel token={detail.cancelToken} />
      ) : (
        <CancelUnavailable />
      )}
    </main>
  )
}

export function BookingCancelPage() {
  const t = useTranslations("bookingWebview")
  const token = useToken()
  const detail = useBookingPage<ScheduleDetail>("bookingCancel", token)
  if (!detail) {
    return <InvalidLink />
  }
  return (
    <main className="mx-auto flex min-h-screen w-full max-w-xl flex-col justify-center gap-6 p-6">
      <div className="space-y-2">
        <h1 className="font-semibold text-3xl tracking-normal">
          {t("cancel.title")}
        </h1>
        <p className="text-muted-foreground">
          {t("cancel.description", {
            calendar: detail.calendarName,
            time: formatDateTime(
              detail.startAt,
              detail.inviteeTimezone,
              "medium",
            ),
          })}
        </p>
      </div>
      {detail.cancellable ? (
        <CancelAppointmentPanel token={token} />
      ) : (
        <CancelUnavailable />
      )}
    </main>
  )
}
