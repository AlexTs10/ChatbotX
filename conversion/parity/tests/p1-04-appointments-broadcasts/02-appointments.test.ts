// appointments.* (book, list, get, cancel, delete, createBookingLink).

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import { Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  type Appointment,
  bookAppointment,
  type CalendarForEdit,
  type Contact,
  createConfiguredCalendar,
  createContact,
  createWebchat,
  errorShape,
  expectError,
  expectInvalid,
  ext,
  extraToken,
  FIXED_TIME_KEYS,
  newTenant,
} from "./helpers.ts"
import { apiClient } from "../../src/http.ts"

const SLOT_0900 = "2030-01-07T09:00:00.000Z"
const SLOT_0930 = "2030-01-07T09:30:00.000Z"

type Setup = {
  tenant: Tenant
  api: Api
  calendar: CalendarForEdit
  contact: Contact
}

const setup = async (label: string, calendarOverrides: Record<string, unknown> = {}): Promise<Setup> => {
  const { tenant, api } = await newTenant(label)
  const webchat = await createWebchat(api)
  const contact = await createContact(api, tenant, { inboxId: webchat.inboxId, firstName: "Ada" })
  const calendar = await createConfiguredCalendar(api, calendarOverrides)
  return { tenant, api, calendar, contact }
}

const messagesOf = async (api: Api, conversationId: string): Promise<string> => {
  const response = await api.get(`/v1/conversations/${conversationId}/messages`)
  expect(response.status, response.text).toBe(200)
  return response.text
}

describe("appointments.book", () => {
  it("books a slot silently (201) and the slot leaves the availability", async () => {
    const normalizer = new Normalizer()
    const { api, calendar, contact } = await setup("appt-book")
    const response = await api.post<Appointment>("/v1/appointments", {
      calendarId: calendar.id,
      contactId: contact.contactId,
      startAt: SLOT_0900,
    })
    expect(response.status, response.text).toBe(201)
    expect(response.json).toMatchObject({
      calendarId: calendar.id,
      contactId: contact.contactId,
      conversationId: null,
      startAt: SLOT_0900,
      endAt: SLOT_0930,
      inviteeTimezone: "UTC",
      status: "scheduled",
      locationType: "onlineMeeting",
      locationDetail: null,
      externalEventId: null,
      cancelledAt: null,
    })
    expect(Object.keys(response.json).sort()).toEqual(
      [
        "calendarId",
        "cancelledAt",
        "contactId",
        "conversationId",
        "createdAt",
        "endAt",
        "externalEventId",
        "id",
        "inviteeTimezone",
        "locationDetail",
        "locationType",
        "startAt",
        "status",
        "updatedAt",
      ].sort(),
    )
    expect(normalizer.normalize(response.json, { keep: FIXED_TIME_KEYS })).toMatchSnapshot("booked")

    const availability = await api.get<{ slots: Array<{ startAt: string }> }>(
      `/v1/appointment-calendars/${calendar.id}/availability`,
      { query: { startDate: "2030-01-07T00:00:00.000Z", endDate: "2030-01-07T23:59:00.000Z" } },
    )
    expect(availability.json.slots.map((slot) => slot.startAt)).toEqual([
      "2030-01-07T09:30:00.000Z",
      "2030-01-07T10:00:00.000Z",
      "2030-01-07T10:30:00.000Z",
    ])
  })

  it("books with a conversation and inbox: the confirmation flow reaches the conversation", async () => {
    const { api, calendar, contact } = await setup("appt-book-conv")
    const appointment = await bookAppointment(api, {
      calendarId: calendar.id,
      contactId: contact.contactId,
      conversationId: contact.conversationId,
      inboxId: contact.inboxId,
      startAt: SLOT_0930,
      inviteeTimezone: "Asia/Ho_Chi_Minh",
    })
    expect(appointment).toMatchObject({
      conversationId: contact.conversationId,
      inviteeTimezone: "Asia/Ho_Chi_Minh",
      startAt: SLOT_0930,
    })
    // Worker: integration queue `sendFlow` with the calendar's confirmation flow.
    await eventually(
      async () => {
        expect(await messagesOf(api, contact.conversationId)).toContain("Appointment Confirmation - Parity calendar")
      },
      { timeoutMs: 30_000, intervalMs: 500, description: "confirmation message" },
    )
  })

  it("falls back to the calendar time zone for an unknown invitee time zone", async () => {
    const { api, calendar, contact } = await setup("appt-book-tz", { timezone: "Europe/Berlin" })
    // 09:00 Berlin (UTC+1 in January) is 08:00 UTC.
    const appointment = await bookAppointment(api, {
      calendarId: calendar.id,
      contactId: contact.contactId,
      startAt: "2030-01-07T08:00:00.000Z",
      inviteeTimezone: "GMT+7",
    })
    expect(appointment.inviteeTimezone).toBe("Europe/Berlin")
  })

  it("rejects bad input with 422", async () => {
    const { api, calendar, contact } = await setup("appt-book-invalid")
    const response = await api.post("/v1/appointments", {})
    expectError(response, 422, "invalidRequestData")
    expect(errorShape(response)).toMatchSnapshot("422 shape")
    for (const path of ["calendarId", "contactId", "startAt"]) {
      expectInvalid(response, path)
    }
    expectInvalid(
      await api.post("/v1/appointments", {
        calendarId: calendar.id,
        contactId: contact.contactId,
        startAt: SLOT_0900,
        inboxId: contact.inboxId,
      }),
      "inboxId",
    )
    expectInvalid(
      await api.post("/v1/appointments", { calendarId: "abc", contactId: contact.contactId, startAt: SLOT_0900 }),
      "calendarId",
    )
    expectInvalid(
      await api.post("/v1/appointments", { calendarId: calendar.id, contactId: contact.contactId, startAt: "soon" }),
      "startAt",
    )
  })

  it("enforces slots, capacity and one booking per contact and start (409)", async () => {
    const { tenant, api, calendar, contact } = await setup("appt-book-conflict")
    const book = (input: Record<string, unknown>) =>
      api.post("/v1/appointments", { calendarId: calendar.id, contactId: contact.contactId, ...input })

    const outside = await book({ startAt: "2030-01-07T08:00:00.000Z" })
    const slotBody = expectError(outside, 409, "slotUnavailable")
    expect(slotBody.message).toBe("Appointment slot is unavailable")
    expect(errorShape(outside)).toMatchSnapshot("409 slotUnavailable")
    expectError(await book({ startAt: "2030-01-07T09:10:00.000Z" }), 409, "slotUnavailable")

    await bookAppointment(api, { calendarId: calendar.id, contactId: contact.contactId, startAt: SLOT_0900 })
    const again = await book({ startAt: SLOT_0900 })
    const againBody = expectError(again, 409, "appointmentAlreadyScheduled")
    expect(againBody.message).toBe("Contact already has a scheduled appointment for this calendar")

    // Capacity 1: another contact cannot take the same slot.
    const webchat = await createWebchat(api, { name: "Second webchat" })
    const other = await createContact(api, tenant, { inboxId: webchat.inboxId, firstName: "Bob" })
    expectError(
      await api.post("/v1/appointments", { calendarId: calendar.id, contactId: other.contactId, startAt: SLOT_0900 }),
      409,
      "slotUnavailable",
    )

    // An inactive calendar accepts no booking.
    await api.patch(`/v1/appointment-calendars/${calendar.id}/active`, { active: false })
    expectError(await book({ startAt: SLOT_0930 }), 409, "slotUnavailable")
  })

  it("is 404 for an unknown contact or calendar and 400 for another contact's conversation", async () => {
    const { tenant, api, calendar, contact } = await setup("appt-book-404")
    const unknownContact = await api.post("/v1/appointments", {
      calendarId: calendar.id,
      contactId: "1",
      startAt: SLOT_0900,
    })
    expectError(unknownContact, 404, "notFound")
    expect(errorShape(unknownContact)).toMatchSnapshot("unknown contact")

    const unknownCalendar = await api.post("/v1/appointments", {
      calendarId: "1",
      contactId: contact.contactId,
      startAt: SLOT_0900,
    })
    expectError(unknownCalendar, 404, "notFound")

    const webchat = await createWebchat(api, { name: "Other webchat" })
    const other = await createContact(api, tenant, { inboxId: webchat.inboxId, firstName: "Eve" })
    const foreignConversation = await api.post("/v1/appointments", {
      calendarId: calendar.id,
      contactId: contact.contactId,
      conversationId: other.conversationId,
      startAt: SLOT_0900,
    })
    const body = expectError(foreignConversation, 400, "invalidAppointmentConversation")
    expect(body.message).toBe("Appointment conversation does not belong to the contact")

    const unknownConversation = await api.post("/v1/appointments", {
      calendarId: calendar.id,
      contactId: contact.contactId,
      conversationId: "1",
      startAt: SLOT_0900,
    })
    expectError(unknownConversation, 404, "notFound")
  })
})

describe("appointments.list", () => {
  it("lists upcoming appointments newest start first, with schedule links", async () => {
    const normalizer = new Normalizer()
    const { api, calendar, contact } = await setup("appt-list")
    const first = await bookAppointment(api, { calendarId: calendar.id, contactId: contact.contactId, startAt: SLOT_0900 })
    const second = await bookAppointment(api, { calendarId: calendar.id, contactId: contact.contactId, startAt: SLOT_0930 })

    const response = await api.get<{
      data: Array<Appointment & { calendarName: string; scheduleUrl: string; cancellable: boolean; deletable: boolean }>
      pageCount: number
    }>("/v1/appointments")
    expect(response.status, response.text).toBe(200)
    expect(response.json.pageCount).toBe(1)
    expect(response.json.data.map((item) => item.id)).toEqual([second.id, first.id])
    for (const item of response.json.data) {
      expect(item).toMatchObject({ calendarName: "Parity calendar", cancellable: true, deletable: false })
      const url = new URL(item.scheduleUrl)
      expect(`${url.origin}${url.pathname}`).toBe("http://app/booking/schedule")
      // A signed (JWS) or encrypted (JWE) compact token.
      // An opaque signed token (base64url JSON header first).
      expect(url.searchParams.get("token") ?? "").toMatch(/^eyJ\S{20,}$/)
      expect(item).not.toHaveProperty("contactName")
      expect(item).not.toHaveProperty("workspaceId")
    }
    expect(normalizer.normalize(response.json, { keep: FIXED_TIME_KEYS })).toMatchSnapshot("list")
  })

  it("filters by calendar, tab and contact name", async () => {
    const { tenant, api, calendar, contact } = await setup("appt-list-filter")
    const otherCalendar = await createConfiguredCalendar(api, {}, "Other calendar")
    const upcoming = await bookAppointment(api, { calendarId: calendar.id, contactId: contact.contactId, startAt: SLOT_0900 })
    const onOther = await bookAppointment(api, {
      calendarId: otherCalendar.id,
      contactId: contact.contactId,
      startAt: SLOT_0900,
    })
    const { appointmentId: pastId } = await (await ext()).createAppointment({
      workspaceId: tenant.workspaceId,
      calendarId: calendar.id,
      contactId: contact.contactId,
      startAt: new Date("2020-01-06T09:00:00.000Z"),
      endAt: new Date("2020-01-06T09:30:00.000Z"),
      inviteeTimezone: "UTC",
      status: "scheduled",
      locationType: "onlineMeeting",
    })
    const ids = async (query: Record<string, string | number>) => {
      const response = await api.get<{ data: Array<{ id: string; cancellable: boolean; deletable: boolean }> }>(
        "/v1/appointments",
        { query },
      )
      expect(response.status, response.text).toBe(200)
      return response.json.data
    }

    expect((await ids({ calendarId: calendar.id })).map((item) => item.id)).toEqual([upcoming.id])
    expect((await ids({ calendarId: otherCalendar.id })).map((item) => item.id)).toEqual([onOther.id])
    const past = await ids({ tab: "past" })
    expect(past.map((item) => item.id)).toEqual([pastId])
    // A past scheduled appointment can no longer be cancelled, so it can be deleted.
    expect(past[0]).toMatchObject({ cancellable: false, deletable: true })
    expect((await ids({ tab: "next", search: "ada" })).length).toBe(2)
    expect(await ids({ search: "nobody" })).toEqual([])

    expectInvalid(await api.get("/v1/appointments", { query: { tab: "later" } }), "tab")
    expectInvalid(await api.get("/v1/appointments", { query: { calendarId: "abc" } }), "calendarId")
    expectInvalid(await api.get("/v1/appointments", { query: { perPage: 0 } }), "perPage")
  })
})

describe("appointments.get", () => {
  it("returns the booked appointment; 404 for unknown, 422 for non-numeric ids", async () => {
    const { api, calendar, contact } = await setup("appt-get")
    const booked = await bookAppointment(api, { calendarId: calendar.id, contactId: contact.contactId, startAt: SLOT_0900 })
    const response = await api.get<Appointment>(`/v1/appointments/${booked.id}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual(booked)

    const unknown = await api.get("/v1/appointments/1")
    const body = expectError(unknown, 404, "notFound")
    expect(body.message).toBe("Appointment not found")
    expectInvalid(await api.get("/v1/appointments/abc"), "id")
  })
})

describe("appointments.cancel", () => {
  it("cancels an upcoming appointment (200) and frees the slot", async () => {
    const normalizer = new Normalizer()
    const { api, calendar, contact } = await setup("appt-cancel")
    const booked = await bookAppointment(api, { calendarId: calendar.id, contactId: contact.contactId, startAt: SLOT_0900 })
    const response = await api.post<Appointment & { cancelledAt: string }>(`/v1/appointments/${booked.id}/cancel`, {})
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ id: booked.id, status: "cancelled", startAt: SLOT_0900 })
    expect(Date.parse(response.json.cancelledAt)).not.toBeNaN()
    expect(Object.keys(response.json).sort()).toEqual(Object.keys(booked).sort())
    expect(normalizer.normalize(response.json, { keep: FIXED_TIME_KEYS })).toMatchSnapshot("cancelled")

    const listed = await api.get<{ data: Array<{ id: string; status: string; cancellable: boolean; deletable: boolean }> }>(
      "/v1/appointments",
    )
    expect(listed.json.data).toEqual([
      expect.objectContaining({ id: booked.id, status: "cancelled", cancellable: false, deletable: true }),
    ])
    // The slot is free again; the same contact can book it again.
    await bookAppointment(api, { calendarId: calendar.id, contactId: contact.contactId, startAt: SLOT_0900 })
  })

  it("is 409 appointmentNotCancellable twice or in the past, 404 for unknown ids", async () => {
    const { tenant, api, calendar, contact } = await setup("appt-cancel-409")
    const booked = await bookAppointment(api, { calendarId: calendar.id, contactId: contact.contactId, startAt: SLOT_0900 })
    await api.post(`/v1/appointments/${booked.id}/cancel`, {})
    const twice = await api.post(`/v1/appointments/${booked.id}/cancel`, {})
    const body = expectError(twice, 409, "appointmentNotCancellable")
    expect(body.message).toBe("Appointment cannot be cancelled")
    expect(errorShape(twice)).toMatchSnapshot("409 shape")

    const { appointmentId: pastId } = await (await ext()).createAppointment({
      workspaceId: tenant.workspaceId,
      calendarId: calendar.id,
      contactId: contact.contactId,
      startAt: new Date("2020-01-06T09:00:00.000Z"),
      endAt: new Date("2020-01-06T09:30:00.000Z"),
      inviteeTimezone: "UTC",
      status: "scheduled",
      locationType: "onlineMeeting",
    })
    expectError(await api.post(`/v1/appointments/${pastId}/cancel`, {}), 409, "appointmentNotCancellable")
    expectError(await api.post("/v1/appointments/1/cancel", {}), 404, "notFound")
    expectInvalid(await api.post("/v1/appointments/abc/cancel", {}), "id")
  })
})

describe("appointments.delete", () => {
  it("blocks deleting an upcoming appointment, deletes a cancelled or past one (204)", async () => {
    const { tenant, api, calendar, contact } = await setup("appt-delete")
    const booked = await bookAppointment(api, { calendarId: calendar.id, contactId: contact.contactId, startAt: SLOT_0900 })
    const blocked = await api.delete(`/v1/appointments/${booked.id}`)
    const body = expectError(blocked, 409, "appointmentDeleteBlocked")
    expect(body.message).toBe("Cancel upcoming appointments before deleting them")

    await api.post(`/v1/appointments/${booked.id}/cancel`, {})
    const deleted = await api.delete(`/v1/appointments/${booked.id}`)
    expect(deleted.status, deleted.text).toBe(204)
    expect(deleted.text).toBe("")
    expectError(await api.get(`/v1/appointments/${booked.id}`), 404, "notFound")
    expect((await api.get<{ data: unknown[] }>("/v1/appointments")).json.data).toEqual([])
    expectError(await api.delete(`/v1/appointments/${booked.id}`), 404, "notFound")

    const { appointmentId: pastId } = await (await ext()).createAppointment({
      workspaceId: tenant.workspaceId,
      calendarId: calendar.id,
      contactId: contact.contactId,
      startAt: new Date("2020-01-06T09:00:00.000Z"),
      endAt: new Date("2020-01-06T09:30:00.000Z"),
      inviteeTimezone: "UTC",
      status: "scheduled",
      locationType: "onlineMeeting",
    })
    expect((await api.delete(`/v1/appointments/${pastId}`)).status).toBe(204)
    expectError(await api.delete("/v1/appointments/1"), 404, "notFound")
  })
})

describe("appointments.createBookingLink", () => {
  it("returns a 24 h booking picker link without sending (201)", async () => {
    const normalizer = new Normalizer()
    const { api, calendar, contact } = await setup("appt-link")
    const before = Date.now()
    const response = await api.post<{ url: string; expiresAt: string; calendar: unknown; messageId: string | null }>(
      `/v1/conversations/${contact.conversationId}/appointment-booking-link`,
      { calendarId: calendar.id },
    )
    expect(response.status, response.text).toBe(201)
    expect(response.json.calendar).toEqual({ id: calendar.id, name: "Parity calendar" })
    expect(response.json.messageId).toBeNull()
    const url = new URL(response.json.url)
    expect(`${url.origin}${url.pathname}`).toBe("http://app/booking/picker")
    expect([...url.searchParams.keys()]).toEqual(["token"])
    expect(url.searchParams.get("token") ?? "").toMatch(/^eyJ\S{20,}$/)
    const ttl = Date.parse(response.json.expiresAt) - before
    expect(ttl).toBeGreaterThan(24 * 3600_000 - 60_000)
    expect(ttl).toBeLessThan(24 * 3600_000 + 60_000)
    expect(normalizer.normalize(response.json)).toMatchSnapshot("link")
    // Nothing was sent.
    expect(await messagesOf(api, contact.conversationId)).not.toContain("Choose an appointment time")
  })

  it("sends the link as a message with send=true", async () => {
    const { api, calendar, contact } = await setup("appt-link-send")
    const response = await api.post<{ messageId: string | null }>(
      `/v1/conversations/${contact.conversationId}/appointment-booking-link`,
      { calendarId: calendar.id, inboxId: contact.inboxId, send: true },
    )
    expect(response.status, response.text).toBe(201)
    expect(response.json.messageId).toMatch(/^\d+$/)
    const messages = await messagesOf(api, contact.conversationId)
    expect(messages).toContain("Choose an appointment time")
    expect(messages).toContain(response.json.messageId as string)
  })

  it("needs the inbox scope to send, an active calendar and a known conversation", async () => {
    const { tenant, api, calendar, contact } = await setup("appt-link-errors")
    const path = `/v1/conversations/${contact.conversationId}/appointment-booking-link`
    const appointmentsOnly = apiClient(await extraToken(tenant, { scopes: ["appointments"] }))
    const noInbox = await appointmentsOnly.post(path, { calendarId: calendar.id, send: true })
    expect(noInbox.status).toBe(403)
    expect((noInbox.json as { message?: string }).message).toBe("Token is not authorized for the 'inbox' scope")
    // Without send the appointments scope is enough.
    expect((await appointmentsOnly.post(path, { calendarId: calendar.id })).status).toBe(201)

    expectInvalid(await api.post(path, {}), "calendarId")
    expectInvalid(await api.post(path, { calendarId: calendar.id, send: "yes" }), "send")
    expectError(
      await api.post("/v1/conversations/1/appointment-booking-link", { calendarId: calendar.id }),
      404,
      "notFound",
    )
    const unknownInbox = await api.post(path, { calendarId: calendar.id, inboxId: "1" })
    const inboxBody = expectError(unknownInbox, 404, "notFound")
    expect(inboxBody.message).toBe("Inbox not found")
    expectError(await api.post(path, { calendarId: "1" }), 404, "notFound")

    await api.patch(`/v1/appointment-calendars/${calendar.id}/active`, { active: false })
    const inactive = await api.post(path, { calendarId: calendar.id })
    const body = expectError(inactive, 409, "appointmentCalendarInactive")
    expect(body.message).toBe("Appointment calendar is inactive")
  })
})
