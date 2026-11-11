// appointmentCalendars.* (create, list, get, update, setActive, duplicate,
// delete, getAvailability).

import { describe, expect, it } from "vitest"
import { Normalizer, unordered } from "../../src/normalize.ts"
import {
  calendarUpdateBody,
  createCalendar,
  createConfiguredCalendar,
  errorShape,
  expectError,
  expectInvalid,
  FIXED_TIME_KEYS,
  getCalendar,
  newTenant,
  subst,
} from "./helpers.ts"

type CalendarListItem = { id: string; name: string; publicLinkSlug: string; publicUrl: string }

const slugged = <T>(value: T, slugs: string[]): T =>
  subst(value, Object.fromEntries(slugs.map((slug, index) => [slug, `<slug:${index + 1}>`])))

describe("appointmentCalendars.create", () => {
  it("creates an active UTC calendar with a confirmation flow and three default reminders", async () => {
    const normalizer = new Normalizer()
    const { api } = await newTenant("cal-create")
    const created = await api.post<{ id: string }>("/v1/appointment-calendars", { name: "Parity calendar" })
    expect(created.status, created.text).toBe(201)
    expect(Object.keys(created.json)).toEqual(["id"])
    expect(created.json.id).toMatch(/^\d{15,20}$/)

    const calendar = await getCalendar(api, created.json.id)
    const slug = calendar.publicLinkSlug as string
    expect(slug).toMatch(/^\d{15,20}$/)
    expect(calendar).toMatchObject({
      id: created.json.id,
      name: "Parity calendar",
      description: null,
      active: true,
      timezone: "UTC",
      locationType: "onlineMeeting",
      externalConnectionId: null,
      cancellationFlowId: null,
      publicUrl: `http://app/booking/${slug}`,
      availability: [],
    })
    expect(calendar.confirmationFlowId).toMatch(/^\d+$/)
    expect(
      unordered(
        calendar.reminders.map(({ timingValue, timingUnit }) => ({ timingValue, timingUnit })),
        (reminder) => `${reminder.timingUnit}:${reminder.timingValue}`,
      ),
    ).toEqual([
      { timingUnit: "days", timingValue: 1 },
      { timingUnit: "hours", timingValue: 1 },
      { timingUnit: "minutes", timingValue: 10 },
    ])
    const reminderFlowIds = new Set(calendar.reminders.map((reminder) => reminder.flowId))
    expect(reminderFlowIds.size).toBe(1)
    expect(reminderFlowIds.has(calendar.confirmationFlowId as string)).toBe(false)

    // Side effect: the confirmation and reminder flows exist (flows.get).
    const confirmationFlow = await api.get<{ name: string }>(`/v1/flows/${calendar.confirmationFlowId}`)
    expect(confirmationFlow.status, confirmationFlow.text).toBe(200)
    expect(confirmationFlow.json.name).toBe("Booking confirmation - Parity calendar")
    const reminderFlow = await api.get<{ name: string }>(`/v1/flows/${[...reminderFlowIds][0]}`)
    expect(reminderFlow.status, reminderFlow.text).toBe(200)
    expect(reminderFlow.json.name).toBe("Reminder - Parity calendar")

    const reminders = unordered(calendar.reminders, (reminder) => `${reminder.timingUnit}:${reminder.timingValue}`)
    expect(normalizer.normalize(slugged({ ...calendar, reminders }, [slug]))).toMatchSnapshot("created calendar")
  })

  it("trims the name", async () => {
    const { api } = await newTenant("cal-create-trim")
    const id = await createCalendar(api, "  Trimmed calendar  ")
    expect((await getCalendar(api, id)).name).toBe("Trimmed calendar")
  })

  it("rejects a missing, blank or too long name with 422", async () => {
    const { api } = await newTenant("cal-create-invalid")
    expectInvalid(await api.post("/v1/appointment-calendars", {}), "name")
    expectInvalid(await api.post("/v1/appointment-calendars", { name: "   " }), "name")
    expectInvalid(await api.post("/v1/appointment-calendars", { name: "x".repeat(256) }), "name")
    expect(errorShape(await api.post("/v1/appointment-calendars", {}))).toMatchSnapshot("422 shape")
  })

  it("rejects a duplicate name with 409 nameAlreadyExists", async () => {
    const { api } = await newTenant("cal-create-dup")
    await createCalendar(api, "Same name")
    const response = await api.post("/v1/appointment-calendars", { name: "Same name" })
    const body = expectError(response, 409, "nameAlreadyExists")
    expect(body.message).toBe("Calendar name already exists")
    expect(errorShape(response)).toMatchSnapshot("409 shape")
  })
})

describe("appointmentCalendars.list", () => {
  it("lists calendars by name, with search and paging", async () => {
    const normalizer = new Normalizer()
    const { api } = await newTenant("cal-list")
    for (const name of ["Calendar B", "Calendar C", "Calendar A"]) {
      await createCalendar(api, name)
    }

    const all = await api.get<{ data: CalendarListItem[]; pageCount: number }>("/v1/appointment-calendars")
    expect(all.status, all.text).toBe(200)
    expect(all.json.pageCount).toBe(1)
    expect(all.json.data.map((item) => item.name)).toEqual(["Calendar A", "Calendar B", "Calendar C"])
    for (const item of all.json.data) {
      expect(item.publicUrl).toBe(`http://app/booking/${item.publicLinkSlug}`)
      // The list item has no availability/reminders (get only).
      expect(item).not.toHaveProperty("availability")
      expect(item).not.toHaveProperty("reminders")
      expect(item).not.toHaveProperty("workspaceId")
    }
    expect(
      normalizer.normalize(slugged(all.json, all.json.data.map((item) => item.publicLinkSlug))),
    ).toMatchSnapshot("list")

    const search = await api.get<{ data: CalendarListItem[]; pageCount: number }>("/v1/appointment-calendars", {
      query: { search: "b" },
    })
    expect(search.status).toBe(200)
    expect(search.json.data.map((item) => item.name)).toEqual(["Calendar B"])
    expect(search.json.pageCount).toBe(1)

    const page2 = await api.get<{ data: CalendarListItem[]; pageCount: number }>("/v1/appointment-calendars", {
      query: { page: 2, perPage: 2 },
    })
    expect(page2.status).toBe(200)
    expect(page2.json.pageCount).toBe(2)
    expect(page2.json.data.map((item) => item.name)).toEqual(["Calendar C"])
  })

  it("returns an empty page for a new workspace", async () => {
    const { api } = await newTenant("cal-list-empty")
    const response = await api.get("/v1/appointment-calendars")
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ data: [], pageCount: 0 })
  })

  it("rejects bad paging with 422", async () => {
    const { api } = await newTenant("cal-list-invalid")
    expectInvalid(await api.get("/v1/appointment-calendars", { query: { perPage: 51 } }), "perPage")
    expectInvalid(await api.get("/v1/appointment-calendars", { query: { page: 0 } }), "page")
    expectInvalid(await api.get("/v1/appointment-calendars", { query: { perPage: "abc" } }), "perPage")
  })

  it("does not list another workspace's calendars", async () => {
    const { api: other } = await newTenant("cal-list-other")
    await createCalendar(other, "Foreign calendar")
    const { api } = await newTenant("cal-list-own")
    const response = await api.get("/v1/appointment-calendars")
    expect(response.json).toEqual({ data: [], pageCount: 0 })
  })
})

describe("appointmentCalendars.get", () => {
  it("is 404 notFound for an unknown or foreign id and 422 for a non-numeric id", async () => {
    const { api: other } = await newTenant("cal-get-other")
    const foreignId = await createCalendar(other)
    const { api } = await newTenant("cal-get")
    const unknown = await api.get("/v1/appointment-calendars/1")
    const body = expectError(unknown, 404, "notFound")
    expect(body.message).toBe("Appointment calendar not found")
    expect(errorShape(unknown)).toMatchSnapshot("404 shape")
    expectError(await api.get(`/v1/appointment-calendars/${foreignId}`), 404, "notFound")
    expectInvalid(await api.get("/v1/appointment-calendars/abc"), "id")
  })
})

describe("appointmentCalendars.update", () => {
  it("replaces the configuration (204) and get shows it", async () => {
    const normalizer = new Normalizer()
    const { api } = await newTenant("cal-update")
    const calendar = await createConfiguredCalendar(api, {
      description: "  Parity description  ",
      locationType: "inPerson",
      locationDetail: "Room 1",
      bufferAfterMinutes: 15,
      maxAppointmentsPerUser: 2,
      dailyLimitEnabled: true,
      maxPerDay: 5,
      allowGroupMeeting: true,
      maxPerSlot: 3,
      confirmationMessage: "See you",
      availability: [
        { weekday: 3, startMinute: 600, endMinute: 1439 },
        { weekday: 1, startMinute: 540, endMinute: 660 },
        { weekday: 1, startMinute: 0, endMinute: 60 },
      ],
      reminders: [],
    })
    expect(calendar).toMatchObject({
      description: "Parity description",
      locationType: "inPerson",
      locationDetail: "Room 1",
      bufferAfterMinutes: 15,
      maxAppointmentsPerUser: 2,
      dailyLimitEnabled: true,
      maxPerDay: 5,
      allowGroupMeeting: true,
      maxPerSlot: 3,
      confirmationMessage: "See you",
      scheduleWindowType: "dateRange",
      scheduleWindowConfig: {
        scheduleWindowType: "dateRange",
        startDate: "2030-01-07",
        endDate: "2030-01-08",
        minAdvanceDays: 0,
      },
      reminders: [],
    })
    // Availability comes back sorted by weekday, then start.
    expect(
      (calendar.availability as Array<{ weekday: number; startMinute: number; endMinute: number }>).map(
        ({ weekday, startMinute, endMinute }) => ({ weekday, startMinute, endMinute }),
      ),
    ).toEqual([
      { weekday: 1, startMinute: 0, endMinute: 60 },
      { weekday: 1, startMinute: 540, endMinute: 660 },
      { weekday: 3, startMinute: 600, endMinute: 1439 },
    ])
    const slug = calendar.publicLinkSlug as string
    expect(
      normalizer.normalize(slugged(calendar, [slug]), { keep: FIXED_TIME_KEYS }),
    ).toMatchSnapshot("updated calendar")
  })

  it("coerces numeric strings and maps the none selection to null", async () => {
    const { api } = await newTenant("cal-update-coerce")
    const calendar = await createConfiguredCalendar(api, {
      durationMinutes: "45",
      bufferAfterMinutes: "__none__",
      confirmationFlowId: "__none__",
      maxPerDay: "",
    })
    expect(calendar).toMatchObject({ durationMinutes: 45, bufferAfterMinutes: null, confirmationFlowId: null, maxPerDay: null })
  })

  it("rejects invalid configurations with 422", async () => {
    const { api } = await newTenant("cal-update-invalid")
    const id = await createCalendar(api)
    const current = await getCalendar(api, id)
    const put = (overrides: Record<string, unknown>) =>
      api.put(`/v1/appointment-calendars/${id}`, calendarUpdateBody(current, overrides))

    expectInvalid(await put({ dailyLimitEnabled: true, maxPerDay: null }), "maxPerDay")
    expectInvalid(await put({ allowGroupMeeting: true, maxPerSlot: null }), "maxPerSlot")
    expectInvalid(await put({ durationMinutes: 7 }), "durationMinutes")
    expectInvalid(await put({ bufferAfterMinutes: 7 }), "bufferAfterMinutes")
    expectInvalid(await put({ locationType: "moon" }), "locationType")
    expectInvalid(await put({ timezone: "" }), "timezone")
    expectInvalid(await put({ name: "" }), "name")
    expectInvalid(
      await put({ availability: [{ weekday: 1, startMinute: 600, endMinute: 540 }] }),
      "availability.0.endMinute",
    )
    expectInvalid(await put({ availability: [{ weekday: 7, startMinute: 0, endMinute: 60 }] }), "availability.0.weekday")
    expectInvalid(await put({ availability: [{ weekday: 1, startMinute: 10, endMinute: 60 }] }), "availability.0.startMinute")
    expectInvalid(
      await put({
        availability: Array.from({ length: 11 }, (_, index) => ({
          weekday: 2,
          startMinute: index * 60,
          endMinute: index * 60 + 30,
        })),
      }),
      "availability.0",
    )
    const reminder = { flowId: current.reminders[0]?.flowId, timingValue: 5, timingUnit: "minutes" }
    expectInvalid(await put({ reminders: [reminder, reminder] }), "reminders.1")
    expectInvalid(await put({ reminders: [{ ...reminder, timingUnit: "weeks" }] }), "reminders.0.timingUnit")
    expectInvalid(
      await put({ scheduleWindowConfig: { scheduleWindowType: "dateRange", startDate: "2030-01-08", endDate: "2030-01-07" } }),
      "scheduleWindowConfig.endDate",
    )
    expectInvalid(await put({ scheduleWindowConfig: { scheduleWindowType: "rollingDays", rollingDays: 366 } }), "scheduleWindowConfig.rollingDays")

    // Nothing was written.
    expect((await getCalendar(api, id)).availability).toEqual([])
  })

  it("is 404 for an unknown calendar or external connection, and checks the flows", async () => {
    const { api } = await newTenant("cal-update-404")
    const id = await createCalendar(api)
    const current = await getCalendar(api, id)

    const unknownCalendar = await api.put("/v1/appointment-calendars/1", calendarUpdateBody(current))
    expectError(unknownCalendar, 404, "notFound")

    const unknownConnection = await api.put(
      `/v1/appointment-calendars/${id}`,
      calendarUpdateBody(current, { externalConnectionId: "1" }),
    )
    const body = expectError(unknownConnection, 404, "notFound")
    expect(body.message).toBe("Google Calendar connection not found")

    const unknownFlow = await api.put(
      `/v1/appointment-calendars/${id}`,
      calendarUpdateBody(current, { cancellationFlowId: "1" }),
    )
    expect(unknownFlow.status).toBeGreaterThanOrEqual(400)
    expect(errorShape(unknownFlow)).toMatchSnapshot("unknown flow error")
  })

  it("rejects a rename onto another calendar's name with 409", async () => {
    const { api } = await newTenant("cal-update-dup")
    await createCalendar(api, "Taken")
    const id = await createCalendar(api, "Mine")
    const current = await getCalendar(api, id)
    const response = await api.put(`/v1/appointment-calendars/${id}`, calendarUpdateBody(current, { name: "Taken" }))
    expectError(response, 409, "nameAlreadyExists")
  })
})

describe("appointmentCalendars.setActive", () => {
  it("deactivates and activates a calendar (204)", async () => {
    const { api } = await newTenant("cal-active")
    const id = await createCalendar(api)
    const off = await api.patch(`/v1/appointment-calendars/${id}/active`, { active: false })
    expect(off.status, off.text).toBe(204)
    expect(off.text).toBe("")
    expect((await getCalendar(api, id)).active).toBe(false)
    const on = await api.patch(`/v1/appointment-calendars/${id}/active`, { active: true })
    expect(on.status).toBe(204)
    expect((await getCalendar(api, id)).active).toBe(true)
  })

  it("is 422 without `active` and 404 for an unknown calendar", async () => {
    const { api } = await newTenant("cal-active-invalid")
    const id = await createCalendar(api)
    expectInvalid(await api.patch(`/v1/appointment-calendars/${id}/active`, {}), "active")
    expectInvalid(await api.patch(`/v1/appointment-calendars/${id}/active`, { active: "yes" }), "active")
    const unknown = await api.patch("/v1/appointment-calendars/1/active", { active: false })
    expectError(unknown, 404, "notFound")
  })
})

describe("appointmentCalendars.duplicate", () => {
  it("copies the configuration into a new inactive calendar (201)", async () => {
    const normalizer = new Normalizer()
    const { api } = await newTenant("cal-duplicate")
    const source = await createConfiguredCalendar(api, { locationDetail: "Room 2", bufferAfterMinutes: 30 }, "Source")
    const response = await api.post<{ id: string }>(`/v1/appointment-calendars/${source.id}/duplicate`, {})
    expect(response.status, response.text).toBe(201)
    expect(Object.keys(response.json)).toEqual(["id"])
    expect(response.json.id).not.toBe(source.id)

    const copy = await getCalendar(api, response.json.id)
    expect(copy.active).toBe(false)
    expect(copy.name).not.toBe("Source")
    expect(copy.publicLinkSlug).not.toBe(source.publicLinkSlug)
    const strip = (calendar: Record<string, unknown>) => ({
      locationDetail: calendar.locationDetail,
      bufferAfterMinutes: calendar.bufferAfterMinutes,
      durationMinutes: calendar.durationMinutes,
      scheduleWindowConfig: calendar.scheduleWindowConfig,
      confirmationFlowId: calendar.confirmationFlowId,
      availability: (calendar.availability as Array<Record<string, unknown>>).map(
        ({ weekday, startMinute, endMinute }) => ({ weekday, startMinute, endMinute }),
      ),
      reminders: unordered(
        (calendar.reminders as Array<Record<string, unknown>>).map(({ flowId, timingValue, timingUnit }) => ({
          flowId,
          timingValue,
          timingUnit,
        })),
        (reminder) => `${reminder.timingUnit}:${reminder.timingValue}`,
      ),
    })
    expect(strip(copy)).toEqual(strip(source))

    // A second copy gets another free name.
    const second = await api.post<{ id: string }>(`/v1/appointment-calendars/${source.id}/duplicate`, {})
    expect(second.status).toBe(201)
    const secondCopy = await getCalendar(api, second.json.id)
    expect(secondCopy.name).not.toBe(copy.name)
    expect({ first: copy.name, second: secondCopy.name }).toMatchSnapshot("copy names")

    const reminders = unordered(copy.reminders, (reminder) => `${reminder.timingUnit}:${reminder.timingValue}`)
    expect(
      normalizer.normalize(slugged({ ...copy, reminders }, [copy.publicLinkSlug as string]), { keep: FIXED_TIME_KEYS }),
    ).toMatchSnapshot("copy")
  })

  it("is 404 for an unknown calendar", async () => {
    const { api } = await newTenant("cal-duplicate-404")
    expectError(await api.post("/v1/appointment-calendars/1/duplicate", {}), 404, "notFound")
    expectInvalid(await api.post("/v1/appointment-calendars/abc/duplicate", {}), "id")
  })
})

describe("appointmentCalendars.delete", () => {
  it("deletes a calendar (204): get is 404, list leaves it out, the name is free again", async () => {
    const { api } = await newTenant("cal-delete")
    const id = await createCalendar(api, "To delete")
    const keep = await createCalendar(api, "To keep")
    const response = await api.delete(`/v1/appointment-calendars/${id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectError(await api.get(`/v1/appointment-calendars/${id}`), 404, "notFound")
    const list = await api.get<{ data: CalendarListItem[] }>("/v1/appointment-calendars")
    expect(list.json.data.map((item) => item.id)).toEqual([keep])
    // Soft delete: the unique name index ignores deleted rows.
    await createCalendar(api, "To delete")

    const again = await api.delete(`/v1/appointment-calendars/${id}`)
    expect({ status: again.status, body: again.json ?? null }).toMatchSnapshot("delete twice")
  })

  it("is 422 for a non-numeric id", async () => {
    const { api } = await newTenant("cal-delete-invalid")
    expectInvalid(await api.delete("/v1/appointment-calendars/abc"), "id")
  })
})

describe("appointmentCalendars.getAvailability", () => {
  const range = { startDate: "2030-01-07T00:00:00.000Z", endDate: "2030-01-08T23:59:59.000Z" }

  it("returns the bookable slots and their text", async () => {
    const { api } = await newTenant("cal-availability")
    const calendar = await createConfiguredCalendar(api)
    const response = await api.get(`/v1/appointment-calendars/${calendar.id}/availability`, { query: range })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      text: "2030-01-07 09:00:00 AM, 2030-01-07 09:30:00 AM, 2030-01-07 10:00:00 AM, 2030-01-07 10:30:00 AM",
      slots: [
        { startAt: "2030-01-07T09:00:00.000Z", endAt: "2030-01-07T09:30:00.000Z" },
        { startAt: "2030-01-07T09:30:00.000Z", endAt: "2030-01-07T10:00:00.000Z" },
        { startAt: "2030-01-07T10:00:00.000Z", endAt: "2030-01-07T10:30:00.000Z" },
        { startAt: "2030-01-07T10:30:00.000Z", endAt: "2030-01-07T11:00:00.000Z" },
      ],
    })
  })

  it("applies the buffer, the time zone and the requested window", async () => {
    const { api } = await newTenant("cal-availability-buffer")
    const calendar = await createConfiguredCalendar(api, {
      bufferAfterMinutes: 30,
      timezone: "Asia/Ho_Chi_Minh",
    })
    const response = await api.get<{ text: string; slots: unknown[] }>(
      `/v1/appointment-calendars/${calendar.id}/availability`,
      { query: range },
    )
    expect(response.status).toBe(200)
    // 09:00 and 10:00 local (UTC+7) on Monday 2030-01-07.
    expect(response.json).toEqual({
      text: "2030-01-07 09:00:00 AM, 2030-01-07 10:00:00 AM",
      slots: [
        { startAt: "2030-01-07T02:00:00.000Z", endAt: "2030-01-07T02:30:00.000Z" },
        { startAt: "2030-01-07T03:00:00.000Z", endAt: "2030-01-07T03:30:00.000Z" },
      ],
    })

    const narrow = await api.get(`/v1/appointment-calendars/${calendar.id}/availability`, {
      query: { startDate: "2030-01-07T02:30:00.000Z", endDate: "2030-01-07T02:59:00.000Z" },
    })
    expect(narrow.json).toEqual({ text: "", slots: [] })
  })

  it("has no slots on an inactive calendar or outside the schedule window", async () => {
    const { api } = await newTenant("cal-availability-inactive")
    const calendar = await createConfiguredCalendar(api)
    const outside = await api.get(`/v1/appointment-calendars/${calendar.id}/availability`, {
      query: { startDate: "2030-01-14T00:00:00.000Z", endDate: "2030-01-14T23:59:00.000Z" },
    })
    expect(outside.json).toEqual({ text: "", slots: [] })
    await api.patch(`/v1/appointment-calendars/${calendar.id}/active`, { active: false })
    const inactive = await api.get(`/v1/appointment-calendars/${calendar.id}/availability`, { query: range })
    expect(inactive.status).toBe(200)
    expect(inactive.json).toEqual({ text: "", slots: [] })
  })

  it("validates the query and is 404 for an unknown calendar", async () => {
    const { api } = await newTenant("cal-availability-invalid")
    const calendar = await createConfiguredCalendar(api)
    const path = `/v1/appointment-calendars/${calendar.id}/availability`
    expectInvalid(await api.get(path, { query: { endDate: range.endDate } }), "startDate")
    expectInvalid(await api.get(path, { query: { startDate: range.startDate } }), "endDate")
    expectInvalid(await api.get(path, { query: { ...range, startDate: "not a date" } }), "startDate")
    expectInvalid(await api.get(path, { query: { ...range, contactId: "abc" } }), "contactId")
    expectError(await api.get("/v1/appointment-calendars/1/availability", { query: range }), 404, "notFound")
  })
})
