// appointmentReminders.list, and the reminder dispatch rows that booking,
// cancelling and editing a calendar write (scheduled worker effects).

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  bookAppointment,
  calendarUpdateBody,
  createCalendar,
  createConfiguredCalendar,
  createContact,
  createWebchat,
  expectInvalid,
  FIXED_TIME_KEYS,
  getCalendar,
  newTenant,
} from "./helpers.ts"

type Dispatch = {
  id: string
  appointmentId: string
  reminderConfigId: string
  contactInboxId: string | null
  sendAt: string
  status: string
  jobId: string
  sentAt: string | null
  cancelledAt: string | null
  failedReason: string | null
}

const listDispatches = async (api: Api, query: Record<string, string | number> = {}) => {
  const response = await api.get<{ data: Dispatch[]; pageCount: number }>("/v1/appointment-reminders", { query })
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("appointmentReminders.list", () => {
  it("is empty for a new workspace", async () => {
    const { api } = await newTenant("rem-empty")
    expect(await listDispatches(api)).toEqual({ data: [], pageCount: 0 })
  })

  it("lists one pending dispatch per calendar reminder of a booking, by send time", async () => {
    const normalizer = new Normalizer()
    const { tenant, api } = await newTenant("rem-book")
    const webchat = await createWebchat(api)
    const contact = await createContact(api, tenant, { inboxId: webchat.inboxId, firstName: "Ada" })
    const calendar = await createConfiguredCalendar(api)
    const appointment = await bookAppointment(api, {
      calendarId: calendar.id,
      contactId: contact.contactId,
      startAt: "2030-01-07T09:00:00.000Z",
    })

    const { data, pageCount } = await listDispatches(api)
    expect(pageCount).toBe(1)
    // Defaults: 1 day, 1 hour and 10 minutes before (calendar in UTC).
    expect(data.map((row) => row.sendAt)).toEqual([
      "2030-01-06T09:00:00.000Z",
      "2030-01-07T08:00:00.000Z",
      "2030-01-07T08:50:00.000Z",
    ])
    const reminderIds = new Set(calendar.reminders.map((reminder) => (reminder as { id: string }).id))
    for (const row of data) {
      expect(row).toMatchObject({
        appointmentId: appointment.id,
        contactInboxId: null,
        status: "pending",
        sentAt: null,
        cancelledAt: null,
        failedReason: null,
        jobId: `appt-reminder-${appointment.id}-${row.reminderConfigId}`,
      })
      expect(reminderIds.has(row.reminderConfigId)).toBe(true)
    }
    const labelled = data.map((row) => ({ ...row, jobId: "<jobId>" }))
    expect(normalizer.normalize({ data: labelled, pageCount }, { keep: FIXED_TIME_KEYS })).toMatchSnapshot("pending")

    // With a conversation and inbox the dispatch records the contact inbox.
    await bookAppointment(api, {
      calendarId: calendar.id,
      contactId: contact.contactId,
      conversationId: contact.conversationId,
      inboxId: contact.inboxId,
      startAt: "2030-01-07T10:00:00.000Z",
    })
    const withInbox = (await listDispatches(api)).data.filter((row) => row.appointmentId !== appointment.id)
    expect(withInbox).toHaveLength(3)
    for (const row of withInbox) {
      expect(row.contactInboxId).toMatch(/^\d+$/)
    }

    // Paging and status filter.
    const page = await listDispatches(api, { perPage: 4, page: 2 })
    expect(page.pageCount).toBe(2)
    expect(page.data).toHaveLength(2)
    expect((await listDispatches(api, { status: "sent" })).data).toEqual([])
    expect((await listDispatches(api, { status: "pending" })).data).toHaveLength(6)
  })

  it("marks the dispatches cancelled when the appointment is cancelled", async () => {
    const { tenant, api } = await newTenant("rem-cancel")
    const webchat = await createWebchat(api)
    const contact = await createContact(api, tenant, { inboxId: webchat.inboxId, firstName: "Ada" })
    const calendar = await createConfiguredCalendar(api)
    const appointment = await bookAppointment(api, {
      calendarId: calendar.id,
      contactId: contact.contactId,
      startAt: "2030-01-07T09:00:00.000Z",
    })
    const cancelled = await api.post(`/v1/appointments/${appointment.id}/cancel`, {})
    expect(cancelled.status).toBe(200)
    const { data } = await listDispatches(api)
    expect(data).toHaveLength(3)
    for (const row of data) {
      expect(row.status).toBe("cancelled")
      expect(Date.parse(row.cancelledAt ?? "")).not.toBeNaN()
    }
    expect((await listDispatches(api, { status: "pending" })).data).toEqual([])
    expect((await listDispatches(api, { status: "cancelled" })).data).toHaveLength(3)
  })

  it("reschedules future dispatches when the calendar's reminders change", async () => {
    const normalizer = new Normalizer()
    const { tenant, api } = await newTenant("rem-reschedule")
    const webchat = await createWebchat(api)
    const contact = await createContact(api, tenant, { inboxId: webchat.inboxId, firstName: "Ada" })
    const calendar = await createConfiguredCalendar(api)
    await bookAppointment(api, {
      calendarId: calendar.id,
      contactId: contact.contactId,
      startAt: "2030-01-07T09:00:00.000Z",
    })
    const flowId = calendar.reminders[0]?.flowId
    const updated = await api.put(
      `/v1/appointment-calendars/${calendar.id}`,
      calendarUpdateBody(calendar, { reminders: [{ flowId, timingValue: 2, timingUnit: "hours" }] }),
    )
    expect(updated.status, updated.text).toBe(204)
    const { data } = await listDispatches(api)
    const pending = data.filter((row) => row.status === "pending")
    expect(pending.map((row) => row.sendAt)).toEqual(["2030-01-07T07:00:00.000Z"])
    expect(
      normalizer.normalize(
        data.map(({ sendAt, status }) => ({ sendAt, status })),
        { keep: FIXED_TIME_KEYS },
      ),
    ).toMatchSnapshot("after reschedule")
  })

  it("rejects an unknown status and bad paging with 422", async () => {
    const { api } = await newTenant("rem-invalid")
    expectInvalid(await api.get("/v1/appointment-reminders", { query: { status: "done" } }), "status")
    expectInvalid(await api.get("/v1/appointment-reminders", { query: { perPage: 500 } }), "perPage")
  })

  it(
    "the worker sends a due reminder to the conversation, and fails one without a conversation",
    async () => {
      const { tenant, api } = await newTenant("rem-worker")
      const webchat = await createWebchat(api)
      const withConversation = await createContact(api, tenant, { inboxId: webchat.inboxId, firstName: "Ada" })
      const silent = await createContact(api, tenant, { inboxId: webchat.inboxId, firstName: "Bob" })
      const calendarId = await createCalendar(api, "Reminder calendar")
      const current = await getCalendar(api, calendarId)
      const reminderFlowId = current.reminders[0]?.flowId

      // Slot: a 5-minute boundary at least 3 minutes ahead. Reminder: N whole
      // minutes before it, so it is due on the next minute boundary that is at
      // least 20 s ahead.
      const MINUTE = 60_000
      const now = Date.now()
      let slot = Math.ceil((now + 3 * MINUTE) / (5 * MINUTE)) * 5 * MINUTE
      if (new Date(slot).getUTCHours() === 23 && new Date(slot).getUTCMinutes() >= 50) {
        slot += 15 * MINUTE
      }
      const dueAt = Math.ceil((now + 20_000) / MINUTE) * MINUTE
      const minutesBefore = Math.round((slot - dueAt) / MINUTE)

      const updated = await api.put(
        `/v1/appointment-calendars/${calendarId}`,
        calendarUpdateBody(current, {
          durationMinutes: 5,
          allowGroupMeeting: true,
          maxPerSlot: 2,
          scheduleWindowConfig: { scheduleWindowType: "anyFutureDate", minAdvanceDays: 0 },
          availability: [0, 1, 2, 3, 4, 5, 6].map((weekday) => ({ weekday, startMinute: 0, endMinute: 1439 })),
          reminders: [{ flowId: reminderFlowId, timingValue: minutesBefore, timingUnit: "minutes" }],
        }),
      )
      expect(updated.status, updated.text).toBe(204)

      const startAt = new Date(slot).toISOString()
      const sent = await bookAppointment(api, {
        calendarId,
        contactId: withConversation.contactId,
        conversationId: withConversation.conversationId,
        inboxId: withConversation.inboxId,
        startAt,
      })
      const failed = await bookAppointment(api, { calendarId, contactId: silent.contactId, startAt })

      const initial = (await listDispatches(api)).data
      expect(initial.map((row) => row.status)).toEqual(["pending", "pending"])
      expect(new Set(initial.map((row) => row.sendAt))).toEqual(new Set([new Date(dueAt).toISOString()]))

      const settled = await eventually(
        async () => {
          const rows = (await listDispatches(api)).data
          expect(rows.every((row) => row.status !== "pending")).toBe(true)
          return rows
        },
        { timeoutMs: 120_000, intervalMs: 2_000, description: "reminder dispatches settle" },
      )
      const byAppointment = new Map(settled.map((row) => [row.appointmentId, row]))
      expect(byAppointment.get(sent.id)).toMatchObject({ status: "sent", failedReason: null })
      expect(Date.parse(byAppointment.get(sent.id)?.sentAt ?? "")).not.toBeNaN()
      expect(byAppointment.get(failed.id)).toMatchObject({
        status: "failed",
        sentAt: null,
        failedReason: "Appointment has no conversation for reminder flow",
      })

      await eventually(
        async () => {
          const messages = await api.get(`/v1/conversations/${withConversation.conversationId}/messages`)
          expect(messages.text).toContain("Reminder - Reminder calendar")
        },
        { timeoutMs: 30_000, intervalMs: 1_000, description: "reminder message" },
      )
    },
    240_000,
  )
})
