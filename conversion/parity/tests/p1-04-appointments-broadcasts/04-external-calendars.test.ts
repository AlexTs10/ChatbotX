// appointmentExternalCalendars.* (list, disconnect) and the Google Calendar
// calls the appointment routes make through a connection (fake upstream):
// freeBusy for availability, events insert/delete from the worker.

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { fake } from "../../src/fake.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  type Api,
  type Appointment,
  bookAppointment,
  calendarUpdateBody,
  createCalendar,
  createConfiguredCalendar,
  createContact,
  createWebchat,
  errorShape,
  expectError,
  expectInvalid,
  ext,
  getCalendar,
  newTenant,
  sha256Hex,
} from "./helpers.ts"

const GOOGLE = "www.googleapis.com"
const OAUTH = "oauth2.googleapis.com"

type Connection = { integrationId: string; providerCalendarId: string; accessToken: string }

const connect = async (tenant: Tenant, email: string | null = "owner@parity.test"): Promise<Connection> => {
  const providerCalendarId = `parity-${uniqueId()}`
  const accessToken = `ya29.parity-${uniqueId()}`
  const { integrationId } = await (await ext()).createGoogleCalendarConnection({
    workspaceId: tenant.workspaceId,
    providerCalendarId,
    email,
    accessToken,
    refreshToken: `1//parity-${uniqueId()}`,
  })
  return { integrationId, providerCalendarId, accessToken }
}

const scriptFreeBusy = (connection: Connection, busy: Array<{ start: string; end: string }>, status = 200) =>
  fake.script({
    match: { host: GOOGLE, method: "POST", path: "/calendar/v3/freeBusy", bodyIncludes: connection.providerCalendarId },
    respond:
      status === 200
        ? {
            status,
            json: {
              kind: "calendar#freeBusy",
              calendars: { [connection.providerCalendarId]: { busy } },
            },
          }
        : { status, json: { error: { code: status, message: "parity failure" } } },
  })

const freeBusyCalls = async (connection: Connection) =>
  (await fake.requests({ kind: "http", host: GOOGLE, pathPrefix: "/calendar/v3/freeBusy" })).filter((record) =>
    record.bodyText.includes(connection.providerCalendarId),
  )

const listConnections = async (api: Api) => {
  const response = await api.get<{ data: Array<Record<string, unknown>>; pageCount: number }>(
    "/v1/appointment-external-calendars",
  )
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("appointmentExternalCalendars.list", () => {
  it("lists Google connections newest first with labels and usage counts", async () => {
    const normalizer = new Normalizer()
    const { tenant, api } = await newTenant("extcal-list")
    expect(await listConnections(api)).toEqual({ data: [], pageCount: 1 })

    const first = await connect(tenant, "owner@parity.test")
    const second = await connect(tenant, null)
    const listed = await listConnections(api)
    expect(listed.pageCount).toBe(1)
    expect(listed.data.map((item) => item.id)).toEqual([second.integrationId, first.integrationId])
    expect(listed.data[1]).toMatchObject({
      id: first.integrationId,
      providerType: "googleCalendar",
      label: `owner@parity.test (${first.providerCalendarId})`,
      providerCalendarId: first.providerCalendarId,
      email: "owner@parity.test",
      connectedCount: 0,
    })
    expect(listed.data[0]).toMatchObject({ label: second.providerCalendarId, email: null })
    for (const item of listed.data) {
      expect(Object.keys(item).sort()).toEqual(
        ["connectedCount", "createdAt", "email", "id", "label", "providerCalendarId", "providerType", "updatedAt"].sort(),
      )
    }
    const replace = {
      [first.providerCalendarId]: "<calendar:1>",
      [second.providerCalendarId]: "<calendar:2>",
      [`owner@parity.test (${first.providerCalendarId})`]: "owner@parity.test (<calendar:1>)",
    }
    expect(normalizer.normalize(listed, { replace })).toMatchSnapshot("list")

    // Paged in memory.
    const page = await api.get<{ data: unknown[]; pageCount: number }>("/v1/appointment-external-calendars", {
      query: { perPage: 1, page: 2 },
    })
    expect(page.json.pageCount).toBe(2)
    expect(page.json.data).toEqual([listed.data[1]])
    expectInvalid(await api.get("/v1/appointment-external-calendars", { query: { perPage: 0 } }), "perPage")
  })

  it("counts the appointment calendars that use a connection", async () => {
    const { tenant, api } = await newTenant("extcal-count")
    const connection = await connect(tenant)
    await createConfiguredCalendar(api, { externalConnectionId: connection.integrationId }, "Synced A")
    await createConfiguredCalendar(api, { externalConnectionId: connection.integrationId }, "Synced B")
    const calendar = await getCalendar(api, await createCalendar(api, "Not synced"))
    expect(calendar.externalConnectionId).toBeNull()
    const listed = await listConnections(api)
    expect(listed.data).toEqual([expect.objectContaining({ id: connection.integrationId, connectedCount: 2 })])
  })
})

describe("appointmentExternalCalendars.disconnect", () => {
  it("revokes the Google token and deletes the connection (204)", async () => {
    const { tenant, api } = await newTenant("extcal-disconnect")
    const connection = await connect(tenant)
    await fake.script({ match: { host: OAUTH, method: "POST", path: "/revoke" }, respond: { status: 200, json: {} } })

    const response = await api.delete(`/v1/appointment-external-calendars/${connection.integrationId}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expect(await listConnections(api)).toEqual({ data: [], pageCount: 1 })

    const revokes = (await fake.requests({ kind: "http", host: OAUTH, pathPrefix: "/revoke" })).filter(
      (record) => record.query.token === connection.accessToken,
    )
    expect(revokes).toHaveLength(1)
    expect(revokes[0]).toMatchObject({ scheme: "https", method: "POST", path: "/revoke", unmatched: false })

    const again = await api.delete(`/v1/appointment-external-calendars/${connection.integrationId}`)
    const body = expectError(again, 404, "notFound")
    expect(body.message).toBe("Google Calendar connection not found")
  })

  it("still deletes the connection when Google rejects the revoke", async () => {
    const { tenant, api } = await newTenant("extcal-disconnect-fail")
    const connection = await connect(tenant)
    await fake.script({
      match: { host: OAUTH, method: "POST", path: "/revoke", bodyIncludes: "" },
      respond: { status: 400, json: { error: "invalid_token" } },
      times: 1,
    })
    const response = await api.delete(`/v1/appointment-external-calendars/${connection.integrationId}`)
    expect(response.status, response.text).toBe(204)
    expect(await listConnections(api)).toEqual({ data: [], pageCount: 1 })
  })

  it("is 409 connectionInUse while a calendar uses it, 404 for unknown ids", async () => {
    const { tenant, api } = await newTenant("extcal-in-use")
    const connection = await connect(tenant)
    await createConfiguredCalendar(api, { externalConnectionId: connection.integrationId })
    const response = await api.delete(`/v1/appointment-external-calendars/${connection.integrationId}`)
    const body = expectError(response, 409, "connectionInUse")
    expect(body.message).toBe("Connection is in use")
    expect(errorShape(response)).toMatchSnapshot("409 shape")
    // No revoke was attempted.
    const revokes = (await fake.requests({ kind: "http", host: OAUTH, pathPrefix: "/revoke" })).filter(
      (record) => record.query.token === connection.accessToken,
    )
    expect(revokes).toEqual([])

    expectError(await api.delete("/v1/appointment-external-calendars/1"), 404, "notFound")
    expectInvalid(await api.delete("/v1/appointment-external-calendars/abc"), "integrationId")
    // Another workspace's connection is not visible.
    const { api: otherApi } = await newTenant("extcal-in-use-other")
    expectError(
      await otherApi.delete(`/v1/appointment-external-calendars/${connection.integrationId}`),
      404,
      "notFound",
    )
  })
})

describe("Google Calendar effects of the appointment routes", () => {
  const range = { startDate: "2030-01-07T00:00:00.000Z", endDate: "2030-01-07T23:59:00.000Z" }

  it("getAvailability removes the slots Google reports busy (freeBusy)", async () => {
    const { tenant, api } = await newTenant("extcal-freebusy")
    const connection = await connect(tenant)
    const calendar = await createConfiguredCalendar(api, { externalConnectionId: connection.integrationId })
    expect(calendar.externalConnectionId).toBe(connection.integrationId)
    await scriptFreeBusy(connection, [{ start: "2030-01-07T09:30:00Z", end: "2030-01-07T10:00:00Z" }])

    const response = await api.get<{ slots: Array<{ startAt: string }> }>(
      `/v1/appointment-calendars/${calendar.id}/availability`,
      { query: range },
    )
    expect(response.status, response.text).toBe(200)
    expect(response.json.slots.map((slot) => slot.startAt)).toEqual([
      "2030-01-07T09:00:00.000Z",
      "2030-01-07T10:00:00.000Z",
      "2030-01-07T10:30:00.000Z",
    ])

    const calls = await freeBusyCalls(connection)
    expect(calls).toHaveLength(1)
    const [call] = calls
    expect(call).toMatchObject({ scheme: "https", method: "POST", unmatched: false })
    expect(call?.headers.authorization).toBe(`Bearer ${connection.accessToken}`)
    const sent = JSON.parse(call?.bodyText ?? "{}") as Record<string, unknown>
    expect(sent).toEqual({
      timeMin: "2030-01-07T00:00:00.000Z",
      timeMax: "2030-01-07T23:59:00.000Z",
      timeZone: "UTC",
      items: [{ id: connection.providerCalendarId }],
    })
  })

  it("getAvailability lists no slots when Google fails", async () => {
    const { tenant, api } = await newTenant("extcal-freebusy-fail")
    const connection = await connect(tenant)
    const calendar = await createConfiguredCalendar(api, { externalConnectionId: connection.integrationId })
    await scriptFreeBusy(connection, [], 500)
    const response = await api.get(`/v1/appointment-calendars/${calendar.id}/availability`, { query: range })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ text: "", slots: [] })
    expect((await freeBusyCalls(connection)).length).toBeGreaterThanOrEqual(1)
  })

  it("a booking makes the worker create the Google event; a cancel deletes it", async () => {
    const { tenant, api } = await newTenant("extcal-sync")
    const connection = await connect(tenant)
    const webchat = await createWebchat(api)
    const contact = await createContact(api, tenant, {
      inboxId: webchat.inboxId,
      firstName: "Ada",
      email: "ada@parity.test",
    })
    const current = await getCalendar(api, await createCalendar(api, "Synced calendar"))
    const updated = await api.put(
      `/v1/appointment-calendars/${current.id}`,
      calendarUpdateBody(current, { externalConnectionId: connection.integrationId, reminders: [] }),
    )
    expect(updated.status, updated.text).toBe(204)
    await scriptFreeBusy(connection, [])
    const eventsPath = `/calendar/v3/calendars/${connection.providerCalendarId}/events`
    await fake.script(
      {
        match: { host: GOOGLE, method: "POST", path: eventsPath },
        respond: { status: 200, json: { id: "placeholder", status: "confirmed" } },
      },
      {
        match: { host: GOOGLE, method: "DELETE", pathPrefix: `${eventsPath}/` },
        respond: { status: 204 },
      },
    )

    const appointment = await bookAppointment(api, {
      calendarId: current.id,
      contactId: contact.contactId,
      conversationId: contact.conversationId,
      inboxId: contact.inboxId,
      startAt: "2030-01-07T09:00:00.000Z",
    })
    expect(appointment.externalEventId).toBeNull()
    // Two freeBusy checks: the availability context and the re-check in the booking transaction.
    expect((await freeBusyCalls(connection)).length).toBe(2)

    const eventId = `abe${sha256Hex(appointment.id)}`
    const synced = await eventually(
      async () => {
        const response = await api.get<Appointment>(`/v1/appointments/${appointment.id}`)
        expect(response.json.externalEventId).toBe(eventId)
        return response.json
      },
      { timeoutMs: 30_000, intervalMs: 500, description: "external event id" },
    )
    expect(synced.status).toBe("scheduled")

    const [insert] = await fake.requests({ kind: "http", host: GOOGLE, method: "POST", pathPrefix: eventsPath })
    expect(insert).toMatchObject({ scheme: "https", method: "POST", path: eventsPath, unmatched: false })
    expect(insert?.query).toMatchObject({ sendUpdates: "all" })
    expect(insert?.headers.authorization).toBe(`Bearer ${connection.accessToken}`)
    const event = JSON.parse(insert?.bodyText ?? "{}") as Record<string, unknown>
    expect(event).toMatchObject({
      id: eventId,
      summary: "Appointment: Synced calendar",
      start: { dateTime: "2030-01-07T09:00:00.000Z", timeZone: "UTC" },
      end: { dateTime: "2030-01-07T09:30:00.000Z", timeZone: "UTC" },
      attendees: [{ email: "ada@parity.test" }],
    })
    expect(Object.keys(event).sort()).toMatchSnapshot("event keys")

    const cancelled = await api.post(`/v1/appointments/${appointment.id}/cancel`, {})
    expect(cancelled.status, cancelled.text).toBe(200)
    const deletes = await fake.waitFor(
      { kind: "http", host: GOOGLE, method: "DELETE", pathPrefix: `${eventsPath}/` },
      { timeoutMs: 30_000 },
    )
    expect(deletes).toHaveLength(1)
    expect(deletes[0]).toMatchObject({ path: `${eventsPath}/${eventId}`, unmatched: false })
    expect(deletes[0]?.query).toMatchObject({ sendUpdates: "all" })
  })

  it("a booking fails when Google cannot be reached for the busy check", async () => {
    const { tenant, api } = await newTenant("extcal-book-fail")
    const connection = await connect(tenant)
    const webchat = await createWebchat(api)
    const contact = await createContact(api, tenant, { inboxId: webchat.inboxId, firstName: "Ada" })
    const calendar = await createConfiguredCalendar(api, {
      externalConnectionId: connection.integrationId,
      reminders: [],
    })
    await scriptFreeBusy(connection, [], 500)
    const response = await api.post("/v1/appointments", {
      calendarId: calendar.id,
      contactId: contact.contactId,
      startAt: "2030-01-07T09:00:00.000Z",
    })
    expect(response.status).toBeGreaterThanOrEqual(400)
    expect(errorShape(response)).toMatchSnapshot("booking with Google down")
    expect((await api.get<{ data: unknown[] }>("/v1/appointments")).json.data).toEqual([])
  })
})
