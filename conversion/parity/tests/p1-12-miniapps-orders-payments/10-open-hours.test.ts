// openHours.list / create / update / delete — named open-hours schedules of
// the store. Resources are `{ id, name, ranges }`; the list is sorted by name,
// then id. A delete makes the store settings that used the schedule always
// open again (`general.openHoursId` -> null).

import { describe, expect, it } from "vitest"
import {
  type Api,
  expectError,
  expectInvalid,
  expectNotFound,
  keysOf,
  newTenant,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueName,
} from "./helpers.ts"

type OpenHours = { id: string; name: string; ranges: Array<{ day: number; from: string; to: string }> }

const WEEKDAYS = [1, 2, 3, 4, 5].map((day) => ({ day, from: "09:00", to: "17:00" }))

const create = async (api: Api, body: { name: string; ranges: OpenHours["ranges"] }): Promise<OpenHours> => {
  const response = await api.post<OpenHours>("/v1/open-hours", body)
  expect(response.status, response.text).toBe(200)
  return response.json
}

const list = async (api: Api): Promise<OpenHours[]> => {
  const response = await api.get<{ data: OpenHours[] }>("/v1/open-hours")
  expect(response.status, response.text).toBe(200)
  expect(keysOf(response.json)).toEqual(["data"])
  return response.json.data
}

describe("openHours.create", () => {
  it("creates a schedule and returns exactly id, name and ranges", async () => {
    const { api } = await newTenant("p112-oh-create")
    const name = uniqueName("Weekdays")
    const response = await api.post<OpenHours>("/v1/open-hours", {
      name: `  ${name}  `,
      ranges: [...WEEKDAYS, { day: 6, from: "22:00", to: "02:00" }],
    })
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(["id", "name", "ranges"])
    expect(response.json.id).toMatch(SNOWFLAKE)
    // The name is trimmed; a range that ends before it starts runs past midnight.
    expect(response.json).toEqual({
      id: response.json.id,
      name,
      ranges: [...WEEKDAYS, { day: 6, from: "22:00", to: "02:00" }],
    })
  })

  it("rejects a missing name, empty ranges, a bad day, a bad time and a zero-length range", async () => {
    const { api } = await newTenant("p112-oh-invalid")
    expectInvalid(await api.post("/v1/open-hours", { ranges: WEEKDAYS }), "name")
    expectInvalid(await api.post("/v1/open-hours", { name: "  ", ranges: WEEKDAYS }), "name")
    expectInvalid(await api.post("/v1/open-hours", { name: "x", ranges: [] }), "ranges")
    expectInvalid(await api.post("/v1/open-hours", { name: "x", ranges: [{ day: 7, from: "09:00", to: "10:00" }] }), "ranges.0.day")
    expectInvalid(await api.post("/v1/open-hours", { name: "x", ranges: [{ day: 1, from: "9:00", to: "10:00" }] }), "ranges.0.from")
    expectInvalid(await api.post("/v1/open-hours", { name: "x", ranges: [{ day: 1, from: "25:00", to: "10:00" }] }), "ranges.0.from")
    expectInvalid(await api.post("/v1/open-hours", { name: "x", ranges: [{ day: 1, from: "10:00", to: "10:00" }] }), "ranges.0")
    // At most 28 ranges (7 days x 4).
    const many = Array.from({ length: 29 }, (_, index) => ({ day: index % 7, from: "08:00", to: "09:00" }))
    expectInvalid(await api.post("/v1/open-hours", { name: "x", ranges: many }), "ranges")
    expect(await list(api)).toEqual([])
  })
})

describe("openHours.list", () => {
  it("lists only this workspace's schedules, sorted by name then id", async () => {
    const { api } = await newTenant("p112-oh-list")
    const { api: other } = await newTenant("p112-oh-list-other")
    expect(await list(api)).toEqual([])
    const zeta = await create(api, { name: "Zeta hours", ranges: WEEKDAYS })
    const alpha = await create(api, { name: "Alpha hours", ranges: [{ day: 0, from: "10:00", to: "12:00" }] })
    const alpha2 = await create(api, { name: "Alpha hours", ranges: [{ day: 6, from: "10:00", to: "12:00" }] })
    await create(other, { name: "Other workspace", ranges: WEEKDAYS })
    // Two schedules may share a name; the older id comes first.
    expect(await list(api)).toEqual([alpha, alpha2, zeta])
  })
})

describe("openHours.update", () => {
  it("changes only the sent fields; ranges are replaced as a whole", async () => {
    const { api } = await newTenant("p112-oh-update")
    const schedule = await create(api, { name: uniqueName("Hours"), ranges: WEEKDAYS })

    const renamed = await api.patch<OpenHours>(`/v1/open-hours/${schedule.id}`, { name: "Renamed" })
    expect(renamed.status, renamed.text).toBe(200)
    expect(renamed.json).toEqual({ id: schedule.id, name: "Renamed", ranges: WEEKDAYS })

    const ranges = [{ day: 3, from: "12:00", to: "13:30" }]
    const replaced = await api.patch<OpenHours>(`/v1/open-hours/${schedule.id}`, { ranges })
    expect(replaced.status, replaced.text).toBe(200)
    expect(replaced.json).toEqual({ id: schedule.id, name: "Renamed", ranges })

    // An empty body is a 500 in the original (no column to set); not asserted.
    expect(await list(api)).toEqual([{ id: schedule.id, name: "Renamed", ranges }])
  })

  it("404 for an unknown id or a schedule of another workspace; 422 for bad input", async () => {
    const { api } = await newTenant("p112-oh-update-404")
    const { api: other } = await newTenant("p112-oh-update-404-other")
    const foreign = await create(other, { name: uniqueName("Foreign"), ranges: WEEKDAYS })
    expectNotFound(await api.patch(`/v1/open-hours/${UNKNOWN_ID}`, { name: "x" }), "Open-hours schedule not found")
    expectNotFound(await api.patch(`/v1/open-hours/${foreign.id}`, { name: "x" }), "Open-hours schedule not found")
    expectInvalid(await api.patch("/v1/open-hours/not-a-number", { name: "x" }), "id")
    const mine = await create(api, { name: uniqueName("Mine"), ranges: WEEKDAYS })
    expectInvalid(await api.patch(`/v1/open-hours/${mine.id}`, { ranges: [] }), "ranges")
    expectInvalid(await api.patch(`/v1/open-hours/${mine.id}`, { name: "" }), "name")
    expect((await list(other))[0]?.name).toBe(foreign.name)
  })
})

describe("openHours.delete", () => {
  it("204 with no body; the store settings that used it become always open", async () => {
    const { api } = await newTenant("p112-oh-delete")
    const schedule = await create(api, { name: uniqueName("Hours"), ranges: WEEKDAYS })
    const keep = await create(api, { name: uniqueName("Keep"), ranges: WEEKDAYS })
    const settings = await api.patch<{ general: { openHoursId: string | null } }>("/v1/ecommerce/settings", {
      general: { openHoursId: schedule.id },
    })
    expect(settings.status, settings.text).toBe(200)
    expect(settings.json.general.openHoursId).toBe(schedule.id)

    const response = await api.delete(`/v1/open-hours/${schedule.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expect(await list(api)).toEqual([keep])
    const after = await api.get<{ general: { openHoursId: string | null } }>("/v1/ecommerce/settings")
    expect(after.json.general.openHoursId).toBeNull()

    // A second delete of the same id is a 404.
    expectNotFound(await api.delete(`/v1/open-hours/${schedule.id}`), "Open-hours schedule not found")
  })

  it("404 for an unknown id or a schedule of another workspace", async () => {
    const { api } = await newTenant("p112-oh-delete-404")
    const { api: other } = await newTenant("p112-oh-delete-404-other")
    const foreign = await create(other, { name: uniqueName("Foreign"), ranges: WEEKDAYS })
    expectNotFound(await api.delete(`/v1/open-hours/${UNKNOWN_ID}`), "Open-hours schedule not found")
    expectNotFound(await api.delete(`/v1/open-hours/${foreign.id}`), "Open-hours schedule not found")
    expect(await list(other)).toEqual([foreign])
    expectError(await api.delete("/v1/open-hours/abc"), 422, "invalidRequestData")
  })
})
