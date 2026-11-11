// broadcasts.* one-off lifecycle without sending: create, list, get, update,
// updateDraft, schedule, moveToDraft, duplicate, clone, delete,
// previewAudience, getAudience, listContacts. Sending is in 13-*.

import { describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  type Broadcast,
  createContact,
  createPublishedFlow,
  createWebchat,
  errorShape,
  expectError,
  expectInvalid,
  newTenant,
} from "./helpers.ts"


const FUTURE = "2030-01-07T09:00:00.000Z"
const KEEP = ["schedulesAt", "nextRunAt", "nextSendAt", "upcomingRuns", "occurrenceAt"]

const setup = async (label: string): Promise<{ tenant: Tenant; api: Api; flowId: string }> => {
  const { tenant, api } = await newTenant(label)
  const flowId = await createPublishedFlow(api, "Parity broadcast flow")
  return { tenant, api, flowId }
}

const body = (flowId: string, overrides: Record<string, unknown> = {}) => ({
  channel: "omnichannel",
  subaction: "allContacts",
  flowId,
  schedulesType: "future",
  schedulesAt: FUTURE,
  contactFilter: { operator: "and", conditions: [] },
  ...overrides,
})

const create = async (api: Api, flowId: string, overrides: Record<string, unknown> = {}): Promise<Broadcast> => {
  const response = await api.post<Broadcast>("/v1/broadcasts", body(flowId, overrides))
  expect(response.status, response.text).toBe(201)
  return response.json
}

const get = async (api: Api, idOrName: string): Promise<Broadcast> => {
  const response = await api.get<Broadcast>(`/v1/broadcasts/${encodeURIComponent(idOrName)}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("broadcasts.create", () => {
  it("creates a draft named after its flow (201)", async () => {
    const normalizer = new Normalizer()
    const { api, flowId } = await setup("bc-create-draft")
    const response = await api.post<Broadcast>("/v1/broadcasts", body(flowId, { saveAsDraft: true }))
    expect(response.status, response.text).toBe(201)
    expect(response.json).toMatchObject({
      name: "Parity broadcast flow",
      status: "draft",
      schedulesType: "future",
      schedulesAt: FUTURE,
      flowId,
      contactCount: null,
      channel: "omnichannel",
      subaction: "allContacts",
      targetMode: "channel",
      sendRatePerMinute: null,
      audienceRangeStart: null,
      audienceRangeEnd: null,
    })
    expect(response.json).not.toHaveProperty("workspaceId")
    expect(normalizer.normalize(response.json, { keep: KEEP })).toMatchSnapshot("created draft")
  })

  it("schedules a future broadcast and keeps the send limits", async () => {
    const normalizer = new Normalizer()
    const { api, flowId } = await setup("bc-create-future")
    const created = await create(api, flowId, {
      schedulesAt: "2030-01-07T09:00:42.123Z",
      sendRatePerMinute: 30,
      audienceRangeStart: 2,
      audienceRangeEnd: 10,
    })
    expect(created).toMatchObject({
      status: "scheduled",
      // Truncated to the minute.
      schedulesAt: FUTURE,
      sendRatePerMinute: 30,
      audienceRangeStart: 2,
      audienceRangeEnd: 10,
    })
    const detail = await get(api, created.id)
    expect(detail).toMatchObject({ id: created.id, status: "scheduled", recurrence: null })
    expect(normalizer.normalize(detail, { keep: KEEP })).toMatchSnapshot("scheduled detail")
  })

  it("rejects payloads that break the schema rules (422 invalidRequestData)", async () => {
    const { api, flowId } = await setup("bc-create-invalid")
    const empty = await api.post("/v1/broadcasts", {})
    for (const path of ["channel", "subaction", "schedulesType", "schedulesAt"]) {
      expectInvalid(empty, path)
    }
    expect(errorShape(empty)).toMatchSnapshot("422 shape")
    const post = (overrides: Record<string, unknown>) => api.post("/v1/broadcasts", body(flowId, overrides))
    expectInvalid(await post({ flowId: undefined }), "flowId")
    expectInvalid(await post({ templateId: "123" }), "flowId")
    expectInvalid(await post({ schedulesType: "recurring" }), "recurrence")
    expectInvalid(await post({ schedulesAt: "2020-01-01T00:00:00.000Z" }), "schedulesAt")
    expectInvalid(await post({ schedulesAt: null }), "schedulesAt")
    expectInvalid(await post({ audienceRangeStart: 5, audienceRangeEnd: 2 }), "audienceRange")
    expectInvalid(await post({ sendRatePerMinute: 0 }), "sendRatePerMinute")
    expectInvalid(await post({ sendRatePerMinute: 1001 }), "sendRatePerMinute")
    expectInvalid(await post({ channel: "fax" }), "channel")
    expectInvalid(await post({ subaction: "everyone" }), "subaction")
    expectInvalid(await post({ flowId: "abc" }), "flowId")
    // A draft may carry a past date.
    expect((await post({ schedulesAt: "2020-01-01T00:00:00.000Z", saveAsDraft: true })).status).toBe(201)
  })

  it("rejects payloads the service refuses (422 validation)", async () => {
    const { api, flowId } = await setup("bc-create-refused")
    const post = (overrides: Record<string, unknown>) => api.post("/v1/broadcasts", body(flowId, overrides))
    const channel = await post({ channel: "webchat" })
    expect(expectError(channel, 422, "validation").message).toBe("Unsupported broadcast channel")
    expect(errorShape(channel)).toMatchSnapshot("422 validation shape")
    expect(expectError(await post({ subaction: "messengerActiveContacts" }), 422, "validation").message).toBe(
      "Unsupported broadcast subaction",
    )
    expect(expectError(await post({ flowId: "1" }), 422, "validation").message).toBe("Flow not found")
    const { api: other } = await newTenant("bc-create-refused-other")
    const foreignFlow = await createPublishedFlow(other, "Foreign flow")
    expect(expectError(await post({ flowId: foreignFlow }), 422, "validation").message).toBe("Flow not found")
  })
})

describe("broadcasts.list", () => {
  it("lists newest first and filters by status, name, channel, schedule window and type", async () => {
    const normalizer = new Normalizer()
    const { api, flowId } = await setup("bc-list")
    const draft = await create(api, flowId, { saveAsDraft: true })
    const scheduled = await create(api, flowId, { schedulesAt: "2030-02-01T00:00:00.000Z" })
    await api.put(`/v1/broadcasts/${scheduled.id}`, { name: "February send" })

    const ids = async (query: Record<string, string | number> = {}) => {
      const response = await api.get<{ data: Broadcast[]; pageCount: number }>("/v1/broadcasts", { query })
      expect(response.status, response.text).toBe(200)
      return response.json.data.map((item) => item.id)
    }
    const all = await api.get<{ data: Broadcast[]; pageCount: number }>("/v1/broadcasts")
    expect(all.json.pageCount).toBe(1)
    expect(all.json.data.map((item) => item.id)).toEqual([scheduled.id, draft.id])
    expect(normalizer.normalize(all.json, { keep: KEEP })).toMatchSnapshot("list")

    expect(await ids({ status: "draft" })).toEqual([draft.id])
    expect(await ids({ status: "sent" })).toEqual([])
    expect(await ids({ name: "february" })).toEqual([scheduled.id])
    expect(await ids({ channel: "omnichannel" })).toHaveLength(2)
    expect(await ids({ channel: "whatsapp" })).toEqual([])
    expect(await ids({ scheduledFrom: "2030-01-15T00:00:00.000Z" })).toEqual([scheduled.id])
    expect(await ids({ scheduledTo: "2030-01-15T00:00:00.000Z" })).toEqual([draft.id])
    expect(await ids({ schedulesType: "recurring" })).toEqual([])
    expect(await ids({ includeRuns: "false" })).toHaveLength(2)
    expect(await ids({ perPage: 1, page: 2 })).toEqual([draft.id])

    const byName = await api.get<{ data: Broadcast[] }>(
      "/v1/broadcasts?sort%5B0%5D%5Bid%5D=name&sort%5B0%5D%5Bdesc%5D=false",
    )
    expect(byName.status, byName.text).toBe(200)
    expect(byName.json.data.map((item) => item.name)).toEqual(["February send", "Parity broadcast flow"])
  })

  it("rejects bad filters with 422", async () => {
    const { api } = await newTenant("bc-list-invalid")
    expectInvalid(await api.get("/v1/broadcasts", { query: { status: "paused" } }), "status")
    expectInvalid(await api.get("/v1/broadcasts", { query: { includeRuns: "yes" } }), "includeRuns")
    expectInvalid(await api.get("/v1/broadcasts", { query: { scheduledFrom: "someday" } }), "scheduledFrom")
    expectInvalid(await api.get("/v1/broadcasts", { query: { name: " " } }), "name")
    expectInvalid(await api.get("/v1/broadcasts", { query: { parentBroadcastId: "x" } }), "parentBroadcastId")
  })
})

describe("broadcasts.get", () => {
  it("finds a broadcast by id or exact name; 404 otherwise", async () => {
    const { api, flowId } = await setup("bc-get")
    const created = await create(api, flowId, { saveAsDraft: true })
    const byId = await get(api, created.id)
    expect(byId).toMatchObject({ id: created.id, name: "Parity broadcast flow", recurrence: null })
    expect(await get(api, "Parity broadcast flow")).toEqual(byId)
    const unknown = await api.get("/v1/broadcasts/Nope")
    expect(expectError(unknown, 404, "notFound").message).toBe("Broadcast not found")
    expect(errorShape(unknown)).toMatchSnapshot("404 shape")
    expectError(await api.get("/v1/broadcasts/1"), 404, "notFound")
  })
})

describe("broadcasts.update", () => {
  it("renames a broadcast and returns it", async () => {
    const { api, flowId } = await setup("bc-update")
    const created = await create(api, flowId, { saveAsDraft: true })
    const response = await api.put<Broadcast>(`/v1/broadcasts/${created.id}`, { name: "  Renamed  " })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ id: created.id, name: "Renamed", status: "draft" })
    expect((await get(api, created.id)).name).toBe("Renamed")
  })

  it("validates the name and is 404 for an unknown broadcast", async () => {
    const { api } = await newTenant("bc-update-invalid")
    expectInvalid(await api.put("/v1/broadcasts/1", { name: "" }), "name")
    expectInvalid(await api.put("/v1/broadcasts/1", { name: "x".repeat(256) }), "name")
    expectInvalid(await api.put("/v1/broadcasts/abc", { name: "x" }), "id")
    const unknown = await api.put("/v1/broadcasts/1", { name: "x" })
    expect(unknown.status).toBe(404)
    expect(errorShape(unknown)).toMatchSnapshot("404 shape")
  })
})

describe("broadcasts.updateDraft", () => {
  it("replaces a draft's payload and schedules it with saveAsDraft false", async () => {
    const { api, flowId } = await setup("bc-updatedraft")
    const otherFlow = await createPublishedFlow(api, "Second flow")
    const draft = await create(api, flowId, { saveAsDraft: true })
    await api.put(`/v1/broadcasts/${draft.id}`, { name: "Kept name" })

    const stillDraft = await api.put(`/v1/broadcasts/${draft.id}/draft`, body(otherFlow, { saveAsDraft: true }))
    expect(stillDraft.status, stillDraft.text).toBe(200)
    expect(stillDraft.json).toEqual({ id: draft.id, status: "draft" })
    expect(await get(api, draft.id)).toMatchObject({ flowId: otherFlow, status: "draft", name: "Kept name" })

    const scheduled = await api.put(
      `/v1/broadcasts/${draft.id}/draft`,
      body(otherFlow, { schedulesAt: "2030-03-01T08:00:00.000Z", sendRatePerMinute: 10 }),
    )
    expect(scheduled.status, scheduled.text).toBe(200)
    expect(scheduled.json).toEqual({ id: draft.id, status: "scheduled" })
    expect(await get(api, draft.id)).toMatchObject({
      status: "scheduled",
      schedulesAt: "2030-03-01T08:00:00.000Z",
      sendRatePerMinute: 10,
    })

    // No longer a draft.
    const notDraft = await api.put(`/v1/broadcasts/${draft.id}/draft`, body(otherFlow, { saveAsDraft: true }))
    expect(expectError(notDraft, 400, "systemError").message).toBe("Broadcast is not a draft")
    expect(errorShape(notDraft)).toMatchSnapshot("not a draft")
  })

  it("validates the payload like create", async () => {
    const { api, flowId } = await setup("bc-updatedraft-invalid")
    const draft = await create(api, flowId, { saveAsDraft: true })
    expectInvalid(await api.put(`/v1/broadcasts/${draft.id}/draft`, {}), "channel")
    expectInvalid(await api.put(`/v1/broadcasts/${draft.id}/draft`, body(flowId, { flowId: undefined })), "flowId")
    expectError(await api.put(`/v1/broadcasts/${draft.id}/draft`, body("1")), 422, "validation")
    expectError(await api.put("/v1/broadcasts/1/draft", body(flowId, { saveAsDraft: true })), 400, "systemError")
  })
})

describe("broadcasts.schedule", () => {
  it("schedules a draft for later and refuses a non-draft", async () => {
    const { api, flowId } = await setup("bc-schedule")
    const draft = await create(api, flowId, { saveAsDraft: true })
    const response = await api.post(`/v1/broadcasts/${draft.id}/schedule`, {
      schedulesType: "future",
      schedulesAt: "2030-04-01T10:30:59.000Z",
      sendRatePerMinute: 5,
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ id: draft.id })
    expect(await get(api, draft.id)).toMatchObject({
      status: "scheduled",
      schedulesType: "future",
      schedulesAt: "2030-04-01T10:30:00.000Z",
      sendRatePerMinute: 5,
    })
    const again = await api.post(`/v1/broadcasts/${draft.id}/schedule`, {
      schedulesType: "future",
      schedulesAt: "2030-04-01T10:30:00.000Z",
    })
    expect(expectError(again, 400, "systemError").message).toBe("Broadcast is not a draft")
  })

  it("validates the schedule (422)", async () => {
    const { api, flowId } = await setup("bc-schedule-invalid")
    const draft = await create(api, flowId, { saveAsDraft: true })
    const path = `/v1/broadcasts/${draft.id}/schedule`
    expectInvalid(await api.post(path, {}), "schedulesType")
    expectInvalid(await api.post(path, { schedulesType: "future", schedulesAt: null }), "schedulesAt")
    expectInvalid(await api.post(path, { schedulesType: "future", schedulesAt: "2020-01-01T00:00:00Z" }), "schedulesAt")
    expectInvalid(
      await api.post(path, {
        schedulesType: "future",
        schedulesAt: FUTURE,
        recurrence: {
          frequency: "daily",
          interval: 1,
          timeOfDay: "09:00",
          timezone: "UTC",
          startsOn: "2030-01-01",
          end: { kind: "never" },
        },
      }),
      "recurrence",
    )
    expectError(await api.post("/v1/broadcasts/1/schedule", { schedulesType: "future", schedulesAt: FUTURE }), 400, "systemError")
  })
})

describe("broadcasts.moveToDraft", () => {
  it("moves a scheduled broadcast back to draft; 400 when it is not scheduled", async () => {
    const { api, flowId } = await setup("bc-movetodraft")
    const scheduled = await create(api, flowId)
    const response = await api.post(`/v1/broadcasts/${scheduled.id}/move-to-draft`, {})
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ id: scheduled.id })
    expect(await get(api, scheduled.id)).toMatchObject({ status: "draft", contactCount: null })

    const again = await api.post(`/v1/broadcasts/${scheduled.id}/move-to-draft`, {})
    expect(again.status).toBe(400)
    expect(errorShape(again)).toMatchSnapshot("not scheduled")
    expectInvalid(await api.post("/v1/broadcasts/abc/move-to-draft", {}), "id")
  })
})

describe("broadcasts.duplicate and broadcasts.clone", () => {
  it("copies into drafts with numbered copy names", async () => {
    const normalizer = new Normalizer()
    const { api, flowId } = await setup("bc-duplicate")
    const source = await create(api, flowId, { sendRatePerMinute: 20 })
    const first = await api.post<Broadcast>(`/v1/broadcasts/${source.id}/duplicate`, {})
    expect(first.status, first.text).toBe(201)
    expect(first.json).toMatchObject({
      name: "Parity broadcast flow (Copy 1)",
      status: "draft",
      flowId,
      schedulesType: "future",
      schedulesAt: FUTURE,
      sendRatePerMinute: 20,
      channel: "omnichannel",
      subaction: "allContacts",
    })
    expect(first.json.id).not.toBe(source.id)
    expect(normalizer.normalize(first.json, { keep: KEEP })).toMatchSnapshot("duplicate")

    const second = await api.post<Broadcast>(`/v1/broadcasts/${source.id}/clone`, {})
    expect(second.status, second.text).toBe(201)
    expect(second.json.name).toBe("Parity broadcast flow (Copy 2)")
    // A copy of a copy keeps one suffix and continues the numbering.
    const third = await api.post<Broadcast>(`/v1/broadcasts/${first.json.id}/duplicate`, {})
    expect(third.json.name).toBe("Parity broadcast flow (Copy 3)")
    // The source is unchanged.
    expect((await get(api, source.id)).status).toBe("scheduled")
  })

  it("is 404 for unknown broadcasts", async () => {
    const { api } = await newTenant("bc-duplicate-404")
    expectError(await api.post("/v1/broadcasts/1/duplicate", {}), 404, "notFound")
    expectError(await api.post("/v1/broadcasts/1/clone", {}), 404, "notFound")
    expectInvalid(await api.post("/v1/broadcasts/x/clone", {}), "id")
  })
})

describe("broadcasts.delete", () => {
  it("soft-deletes (204); get and a second delete are 404", async () => {
    const { api, flowId } = await setup("bc-delete")
    const created = await create(api, flowId, { saveAsDraft: true })
    const response = await api.delete(`/v1/broadcasts/${created.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectError(await api.get(`/v1/broadcasts/${created.id}`), 404, "notFound")
    expect((await api.get<{ data: unknown[] }>("/v1/broadcasts")).json.data).toEqual([])
    const again = await api.delete(`/v1/broadcasts/${created.id}`)
    expect(expectError(again, 404, "notFound").message).toBe("Broadcast not found or cannot be deleted")
    expectInvalid(await api.delete("/v1/broadcasts/abc"), "id")
  })
})

describe("broadcasts.previewAudience", () => {
  it("counts and pages the contact inboxes a broadcast would reach", async () => {
    const normalizer = new Normalizer()
    const { tenant, api } = await newTenant("bc-preview")
    const webchat = await createWebchat(api)
    const ada = await createContact(api, tenant, { inboxId: webchat.inboxId, firstName: "Ada" })
    const bob = await createContact(api, tenant, { inboxId: webchat.inboxId, firstName: "Bob" })

    const none = await api.post("/v1/broadcasts/audience/preview", {})
    expect(none.status, none.text).toBe(200)
    expect(none.json).toMatchObject({ total: 0, data: [] })

    const all = await api.post<{ total: number; data: Array<Record<string, unknown>> }>(
      "/v1/broadcasts/audience/preview",
      { channels: ["omnichannel"], subaction: "allContacts" },
    )
    expect(all.status, all.text).toBe(200)
    expect(all.json.total).toBe(2)
    expect(all.json.data.map((row) => row.contactId)).toEqual([ada.contactId, bob.contactId])
    expect(all.json.data[0]).toMatchObject({
      contactId: ada.contactId,
      firstName: "Ada",
      lastName: "Parity",
      avatar: null,
      channel: "webchat",
      conversationId: ada.conversationId,
    })
    expect(normalizer.normalize(all.json)).toMatchSnapshot("preview")

    const byInbox = await api.post<{ total: number }>("/v1/broadcasts/audience/preview", { inboxIds: [webchat.inboxId] })
    expect(byInbox.json.total).toBe(2)
    const page = await api.post<{ total: number; data: Array<{ contactId: string }> }>(
      "/v1/broadcasts/audience/preview",
      { channels: ["omnichannel"], perPage: 1, page: 2 },
    )
    expect(page.json.total).toBe(2)
    expect(page.json.data.map((row) => row.contactId)).toEqual([bob.contactId])
    const windowed = await api.post<{ total: number; data: Array<{ contactId: string }> }>(
      "/v1/broadcasts/audience/preview",
      { channels: ["omnichannel"], audienceRangeStart: 2 },
    )
    expect(windowed.json.total).toBe(1)
    expect(windowed.json.data.map((row) => row.contactId)).toEqual([bob.contactId])
    const bounded = await api.post("/v1/broadcasts/audience/preview", {
      channels: ["omnichannel"],
      countMode: "bounded",
      perPage: 1,
    })
    expect(bounded.status, bounded.text).toBe(200)
    expect(normalizer.normalize(bounded.json)).toMatchSnapshot("bounded")
  })

  it("rejects bad selectors with 422", async () => {
    const { api } = await newTenant("bc-preview-invalid")
    const path = "/v1/broadcasts/audience/preview"
    expectInvalid(await api.post(path, { perPage: 51 }), "perPage")
    expectInvalid(await api.post(path, { page: 0 }), "page")
    expectInvalid(await api.post(path, { channels: ["fax"] }), "channels.0")
    expectInvalid(await api.post(path, { inboxIds: ["x"] }), "inboxIds.0")
    expectInvalid(await api.post(path, { audienceRangeStart: 3, audienceRangeEnd: 1 }), "audienceRangeEnd")
    expectInvalid(await api.post(path, { countMode: "guess" }), "countMode")
  })
})

describe("broadcasts.getAudience and broadcasts.listContacts", () => {
  it("are empty for a broadcast that has not been prepared", async () => {
    const { api, flowId } = await setup("bc-audience-empty")
    const draft = await create(api, flowId, { saveAsDraft: true })
    const audience = await api.get(`/v1/broadcasts/${draft.id}/audience`)
    expect(audience.status, audience.text).toBe(200)
    expect(audience.json).toEqual({ data: [], pageCount: 0 })
    const byName = await api.get(`/v1/broadcasts/${encodeURIComponent("Parity broadcast flow")}/audience`)
    expect(byName.json).toEqual({ data: [], pageCount: 0 })

    const contacts = await api.get(`/v1/broadcasts/${draft.id}/contacts`, { query: { eventType: "message:sent" } })
    expect(contacts.status, contacts.text).toBe(200)
    expect(contacts.json).toEqual({ data: [], total: 0, pageCount: 0 })
  })

  it("validate their input and are 404 for unknown broadcasts", async () => {
    const { api, flowId } = await setup("bc-audience-invalid")
    const draft = await create(api, flowId, { saveAsDraft: true })
    expectError(await api.get("/v1/broadcasts/Unknown/audience"), 404, "notFound")
    expectInvalid(await api.get(`/v1/broadcasts/${draft.id}/audience`, { query: { page: 0 } }), "page")
    expectInvalid(await api.get(`/v1/broadcasts/${draft.id}/contacts`), "eventType")
    expectInvalid(await api.get(`/v1/broadcasts/${draft.id}/contacts`, { query: { eventType: "message:lost" } }), "eventType")
    expectInvalid(
      await api.get(`/v1/broadcasts/${draft.id}/contacts`, { query: { eventType: "message:sent", perPage: 51 } }),
      "perPage",
    )
    const unknown = await api.get("/v1/broadcasts/1/contacts", { query: { eventType: "message:sent" } })
    expect(expectError(unknown, 404, "notFound").message).toBe("Broadcast not found")
    expectInvalid(await api.get("/v1/broadcasts/x/contacts", { query: { eventType: "message:sent" } }), "id")
  })
})
