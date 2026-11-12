// pipelines.list / get / create / update / delete / duplicate, stages
// (createStage / updateStage / deleteStage / reorderStages), fields
// (listFields / createField / updateField / deleteField), members
// (listMembers / setMember / removeMember) and getStats. A workspace token
// bypasses the pipeline ACL (it acts like an admin). Items are in
// 11-pipeline-items.

import { describe, expect, it } from "vitest"
import {
  type Api,
  expectError,
  expectInvalid,
  expectNotFound,
  expectValidation,
  ISO,
  keysOf,
  newContact,
  newTenant,
  ownerMemberId,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueName,
} from "./helpers.ts"

type Stage = {
  id: string
  name: string
  description: string | null
  textColor: string | null
  backgroundColor: string | null
  position: number
  processing: boolean
}
type Field = { id: string; name: string; referenceName: string; type: string; required: boolean; position: number }
export type Pipeline = {
  id: string
  name: string
  type: string
  referenceName: string
  currency: string
  currencyOverride: string | null
  settings: Record<string, unknown> & { statusLabels: Record<string, string> }
  createdAt: string
  updatedAt: string
  stages: Stage[]
  fields: Field[]
}

const PIPELINE_KEYS = ["createdAt", "currency", "currencyOverride", "id", "name", "referenceName", "settings", "type", "updatedAt"]

const settingsFor = (statusLabels: Record<string, string>) => ({
  version: 1,
  assignContactOnAssign: false,
  maxOpenItemsPerContact: null,
  visibleFeatures: { priority: true, value: true, dueDate: true },
  cardElements: {
    contactPhone: false,
    contactEmail: false,
    createdAt: true,
    updatedAt: false,
    assignees: true,
    comments: true,
    cardId: true,
    priority: true,
    value: true,
    dueDate: true,
  },
  statusLabels,
})

const create = async (api: Api, body: { type?: string; name?: string } = {}): Promise<Pipeline> => {
  const response = await api.post<Pipeline>("/v1/pipelines", { type: "opportunity", name: uniqueName("Sales"), ...body })
  expect(response.status, response.text).toBe(201)
  return response.json
}

const addStage = async (api: Api, pipelineId: string, body: Record<string, unknown>): Promise<Stage> => {
  const response = await api.post<Stage>(`/v1/pipelines/${pipelineId}/stages`, body)
  expect(response.status, response.text).toBe(201)
  return response.json
}

const getPipeline = async (api: Api, id: string): Promise<Pipeline> => {
  const response = await api.get<Pipeline>(`/v1/pipelines/${id}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("pipelines.create / get", () => {
  it("creates each type with its first stage, status labels and default settings", async () => {
    const { api } = await newTenant("p113-pl-create")
    const cases: Array<[string, string, Record<string, string>]> = [
      ["opportunity", "Lead", { open: "Open", won: "Won", lost: "Lost" }],
      ["ticket", "New Issue", { open: "Open", won: "Resolved", lost: "Not Resolved" }],
      ["custom", "Intake", { open: "Open", won: "Resolved", lost: "Not Resolved" }],
    ]
    for (const [type, firstStage, statusLabels] of cases) {
      const response = await api.post<Pipeline>("/v1/pipelines", { type, name: `  ${type} Board ` })
      expect(response.status, response.text).toBe(201)
      const pipeline = response.json
      expect(keysOf(pipeline)).toEqual([...PIPELINE_KEYS, "fields", "stages"].sort())
      expect(pipeline).toEqual({
        id: expect.stringMatching(SNOWFLAKE),
        name: `${type} Board`,
        type,
        referenceName: `${type}_board`,
        currency: "USD",
        currencyOverride: null,
        settings: settingsFor(statusLabels),
        createdAt: expect.stringMatching(ISO),
        updatedAt: expect.stringMatching(ISO),
        stages: [
          {
            id: expect.stringMatching(SNOWFLAKE),
            name: firstStage,
            description: null,
            textColor: null,
            backgroundColor: null,
            position: 0,
            processing: false,
          },
        ],
        fields: [],
      })
      expect(await getPipeline(api, pipeline.id)).toEqual(pipeline)
    }
  })

  it("suffixes a taken name and reference name; 422 for a bad type or name", async () => {
    const { api } = await newTenant("p113-pl-names")
    const first = await create(api, { name: "Deals" })
    const second = await create(api, { name: "Deals" })
    const third = await create(api, { name: "deals" })
    expect([first.name, first.referenceName]).toEqual(["Deals", "deals"])
    expect([second.name, second.referenceName]).toEqual(["Deals 2", "deals_2"])
    expect([third.name, third.referenceName]).toEqual(["deals", "deals_3"])
    expectInvalid(await api.post("/v1/pipelines", { type: "kanban", name: "x" }), "type")
    expectInvalid(await api.post("/v1/pipelines", { type: "custom", name: " " }), "name")
    expectInvalid(await api.post("/v1/pipelines", { type: "custom", name: "x".repeat(121) }), "name")
    expectInvalid(await api.post("/v1/pipelines", { type: "custom", name: "a\u0000b" }), "name")
  })

  it("get 404s for an unknown id or another workspace's pipeline", async () => {
    const { api } = await newTenant("p113-pl-get")
    const { api: other } = await newTenant("p113-pl-get-other")
    const pipeline = await create(other)
    expectNotFound(await api.get(`/v1/pipelines/${pipeline.id}`), "Pipeline not found")
    expectNotFound(await api.get(`/v1/pipelines/${UNKNOWN_ID}`), "Pipeline not found")
    expectInvalid(await api.get("/v1/pipelines/abc"), "id")
  })
})

describe("pipelines.list", () => {
  it("lists this workspace's pipelines oldest first with their item counts", async () => {
    const { api } = await newTenant("p113-pl-list")
    const { api: other } = await newTenant("p113-pl-list-other")
    expect((await api.get("/v1/pipelines")).json).toEqual({ items: [] })
    const first = await create(api)
    const second = await create(api, { type: "ticket" })
    await create(other)
    const contactId = await newContact(api)
    for (let index = 0; index < 2; index += 1) {
      const item = await api.post(`/v1/pipelines/${second.id}/items`, { contactId, title: `Ticket ${index}` })
      expect(item.status, item.text).toBe(201)
    }
    const response = await api.get<{ items: Array<Record<string, unknown>> }>("/v1/pipelines")
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(["items"])
    const { stages: _s1, fields: _f1, ...firstBase } = first
    const { stages: _s2, fields: _f2, ...secondBase } = second
    expect(response.json.items).toEqual([
      { ...firstBase, itemsCount: 0 },
      { ...secondBase, itemsCount: 2 },
    ])
  })
})

describe("pipelines.update", () => {
  it("renames, sets a reference name and currency, merges settings; returns the pipeline without stages", async () => {
    const { api } = await newTenant("p113-pl-update")
    const pipeline = await create(api)
    const response = await api.patch<Pipeline>(`/v1/pipelines/${pipeline.id}`, {
      name: "Renamed board",
      referenceName: "renamed_board",
      currency: "EUR",
      settings: { maxOpenItemsPerContact: 3, cardElements: { contactEmail: true }, statusLabels: { won: "Closed" } },
    })
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(PIPELINE_KEYS)
    const expectedSettings = settingsFor({ open: "Open", won: "Closed", lost: "Lost" })
    expect(response.json).toEqual({
      ...pipeline,
      stages: undefined,
      fields: undefined,
      name: "Renamed board",
      referenceName: "renamed_board",
      currency: "EUR",
      currencyOverride: "EUR",
      settings: {
        ...expectedSettings,
        maxOpenItemsPerContact: 3,
        cardElements: { ...expectedSettings.cardElements, contactEmail: true },
      },
      updatedAt: expect.stringMatching(ISO),
    })
    // currency null follows the workspace again.
    const reset = await api.patch<Pipeline>(`/v1/pipelines/${pipeline.id}`, { currency: null })
    expect(reset.json).toMatchObject({ currency: "USD", currencyOverride: null })
  })

  it("422 for a name or reference name another pipeline uses, a bad reference or currency; 404 unknown", async () => {
    const { api } = await newTenant("p113-pl-update-err")
    const pipeline = await create(api)
    const other = await create(api)
    expectValidation(
      await api.patch(`/v1/pipelines/${pipeline.id}`, { name: other.name }),
      "This name is already used by another pipeline",
    )
    expectValidation(
      await api.patch(`/v1/pipelines/${pipeline.id}`, { referenceName: other.referenceName }),
      "This name is already used by another pipeline",
    )
    // Its own name is free.
    expect((await api.patch(`/v1/pipelines/${pipeline.id}`, { name: pipeline.name })).status).toBe(200)
    expectInvalid(await api.patch(`/v1/pipelines/${pipeline.id}`, { referenceName: "Bad-Name" }), "referenceName")
    expectInvalid(await api.patch(`/v1/pipelines/${pipeline.id}`, { currency: "eur" }), "currency")
    expectInvalid(
      await api.patch(`/v1/pipelines/${pipeline.id}`, { settings: { maxOpenItemsPerContact: 0 } }),
      "settings.maxOpenItemsPerContact",
    )
    expectNotFound(await api.patch(`/v1/pipelines/${UNKNOWN_ID}`, { name: "x" }), "Pipeline not found")
  })
})

describe("pipelines.delete / duplicate", () => {
  it("delete is 204 and removes the pipeline with its items", async () => {
    const { api } = await newTenant("p113-pl-delete")
    const pipeline = await create(api)
    const contactId = await newContact(api)
    expect((await api.post(`/v1/pipelines/${pipeline.id}/items`, { contactId, title: "Gone" })).status).toBe(201)
    const response = await api.delete(`/v1/pipelines/${pipeline.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectNotFound(await api.get(`/v1/pipelines/${pipeline.id}`), "Pipeline not found")
    expect((await api.get(`/v1/contacts/id:${contactId}/pipeline-items`)).json).toEqual({ items: [] })
    expectNotFound(await api.delete(`/v1/pipelines/${pipeline.id}`), "Pipeline not found")
  })

  it("duplicate copies stages and fields (not items) as '<name> copy'", async () => {
    const { api } = await newTenant("p113-pl-duplicate")
    const pipeline = await create(api, { name: "Board" })
    await addStage(api, pipeline.id, { name: "Won stage", description: "Done", textColor: "#FFFFFF", backgroundColor: "#00aa00" })
    const field = await api.post<Field>(`/v1/pipelines/${pipeline.id}/fields`, {
      name: "Budget",
      referenceName: "budget",
      type: "number",
      required: true,
    })
    expect(field.status, field.text).toBe(201)
    const contactId = await newContact(api)
    expect(
      (await api.post(`/v1/pipelines/${pipeline.id}/items`, { contactId, title: "Kept", fieldValues: { [field.json.id]: "5" } }))
        .status,
    ).toBe(201)
    const source = await getPipeline(api, pipeline.id)

    const response = await api.post<Pipeline>(`/v1/pipelines/${pipeline.id}/duplicate`, {})
    expect(response.status, response.text).toBe(201)
    const copy = response.json
    expect(copy.id).not.toBe(pipeline.id)
    expect(copy).toMatchObject({ name: "Board copy", referenceName: "board_copy", type: "opportunity", settings: source.settings })
    expect(copy.stages.map(({ id: _id, ...stage }) => stage)).toEqual(source.stages.map(({ id: _id, ...stage }) => stage))
    expect(copy.fields.map(({ id: _id, ...rest }) => rest)).toEqual(source.fields.map(({ id: _id, ...rest }) => rest))
    expect(copy.stages.map((stage) => stage.id)).not.toContain(source.stages[0]?.id)
    const items = await api.get<{ items: unknown[] }>(`/v1/pipelines/${copy.id}/items`)
    expect(items.json.items).toEqual([])
    const again = await api.post<Pipeline>(`/v1/pipelines/${pipeline.id}/duplicate`, {})
    expect(again.json.name).toBe("Board copy 2")
    expectNotFound(await api.post(`/v1/pipelines/${UNKNOWN_ID}/duplicate`, {}), "Pipeline not found")
  })
})

describe("pipeline stages", () => {
  it("createStage appends with the next position; updateStage changes only the sent fields", async () => {
    const { api } = await newTenant("p113-st-create")
    const pipeline = await create(api)
    const stage = await addStage(api, pipeline.id, { name: "  Qualified  ", description: "Talked to them", textColor: "#112233" })
    expect(keysOf(stage)).toEqual(["backgroundColor", "description", "id", "name", "position", "processing", "textColor"])
    expect(stage).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      name: "Qualified",
      description: "Talked to them",
      textColor: "#112233",
      backgroundColor: null,
      position: 1,
      processing: false,
    })
    const updated = await api.patch<Stage>(`/v1/pipelines/${pipeline.id}/stages/${stage.id}`, {
      backgroundColor: "#ABCDEF",
      description: null,
    })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).toEqual({ ...stage, backgroundColor: "#ABCDEF", description: null })
    expect((await getPipeline(api, pipeline.id)).stages.map((s) => s.name)).toEqual(["Lead", "Qualified"])

    expectInvalid(await api.post(`/v1/pipelines/${pipeline.id}/stages`, { name: "x", textColor: "red" }), "textColor")
    expectInvalid(await api.post(`/v1/pipelines/${pipeline.id}/stages`, {}), "name")
    expectNotFound(await api.patch(`/v1/pipelines/${pipeline.id}/stages/${UNKNOWN_ID}`, { name: "x" }), "Stage not found")
    expectNotFound(await api.post(`/v1/pipelines/${UNKNOWN_ID}/stages`, { name: "x" }), "Pipeline not found")
  })

  it("reorderStages needs every stage exactly once and sets positions in that order", async () => {
    const { api } = await newTenant("p113-st-reorder")
    const pipeline = await create(api)
    const lead = pipeline.stages[0] as Stage
    const second = await addStage(api, pipeline.id, { name: "Second" })
    const third = await addStage(api, pipeline.id, { name: "Third" })
    const response = await api.post(`/v1/pipelines/${pipeline.id}/stages/reorder`, { stageIds: [third.id, lead.id, second.id] })
    expect(response.status, response.text).toBe(204)
    const stages = (await getPipeline(api, pipeline.id)).stages
    expect(stages.map((stage) => [stage.name, stage.position])).toEqual([
      ["Third", 0],
      ["Lead", 1],
      ["Second", 2],
    ])
    const message = "stageIds must list every stage of the pipeline exactly once"
    expectValidation(await api.post(`/v1/pipelines/${pipeline.id}/stages/reorder`, { stageIds: [lead.id, second.id] }), message)
    expectValidation(
      await api.post(`/v1/pipelines/${pipeline.id}/stages/reorder`, { stageIds: [lead.id, second.id, third.id, UNKNOWN_ID] }),
      message,
    )
    expectInvalid(await api.post(`/v1/pipelines/${pipeline.id}/stages/reorder`, { stageIds: [] }), "stageIds")
  })

  it("deleteStage deletes the stage's items, or moves them with moveItemsToStageId; the last stage stays", async () => {
    const { api } = await newTenant("p113-st-delete")
    const pipeline = await create(api)
    const lead = pipeline.stages[0] as Stage
    const keep = await addStage(api, pipeline.id, { name: "Keep" })
    const drop = await addStage(api, pipeline.id, { name: "Drop" })
    const contactId = await newContact(api)
    const moved = await api.post<{ id: string }>(`/v1/pipelines/${pipeline.id}/items`, { contactId, title: "Moves", stageId: lead.id })
    const deleted = await api.post<{ id: string }>(`/v1/pipelines/${pipeline.id}/items`, {
      contactId,
      title: "Deleted",
      stageId: drop.id,
      status: "won",
    })
    expect([moved.status, deleted.status]).toEqual([201, 201])

    const withMove = await api.delete<{ queued: boolean; jobId: string | null }>(`/v1/pipelines/${pipeline.id}/stages/${lead.id}`, {
      json: { moveItemsToStageId: keep.id },
    })
    expect(withMove.status, withMove.text).toBe(200)
    expect(withMove.json).toEqual({ queued: false, jobId: null })
    const movedItem = await api.get<{ stageId: string }>(`/v1/pipelines/${pipeline.id}/items/${moved.json.id}`)
    expect(movedItem.json.stageId).toBe(keep.id)

    const withDelete = await api.delete(`/v1/pipelines/${pipeline.id}/stages/${drop.id}`)
    expect(withDelete.status, withDelete.text).toBe(200)
    expect(withDelete.json).toEqual({ queued: false, jobId: null })
    expectNotFound(await api.get(`/v1/pipelines/${pipeline.id}/items/${deleted.json.id}`), "Item not found")
    expect((await getPipeline(api, pipeline.id)).stages.map((stage) => stage.name)).toEqual(["Keep"])

    expectValidation(await api.delete(`/v1/pipelines/${pipeline.id}/stages/${keep.id}`), "A pipeline needs at least one stage")
    expectNotFound(await api.delete(`/v1/pipelines/${pipeline.id}/stages/${UNKNOWN_ID}`), "Stage not found")
  })

  it("deleteStage refuses moving items into itself or a stage of another pipeline", async () => {
    const { api } = await newTenant("p113-st-delete-move")
    const pipeline = await create(api)
    const other = await create(api)
    const second = await addStage(api, pipeline.id, { name: "Second" })
    const message = "Choose a different stage of this pipeline"
    expectValidation(
      await api.delete(`/v1/pipelines/${pipeline.id}/stages/${second.id}`, { json: { moveItemsToStageId: second.id } }),
      message,
    )
    expectValidation(
      await api.delete(`/v1/pipelines/${pipeline.id}/stages/${second.id}`, {
        json: { moveItemsToStageId: (other.stages[0] as Stage).id },
      }),
      message,
    )
    expect((await getPipeline(api, pipeline.id)).stages).toHaveLength(2)
  })
})

describe("pipeline fields", () => {
  it("createField / listFields / updateField / deleteField", async () => {
    const { api } = await newTenant("p113-fd")
    const pipeline = await create(api)
    const created = await api.post<Field>(`/v1/pipelines/${pipeline.id}/fields`, {
      name: "Source",
      referenceName: "source",
      type: "shortText",
    })
    expect(created.status, created.text).toBe(201)
    expect(keysOf(created.json)).toEqual(["id", "name", "position", "referenceName", "required", "type"])
    expect(created.json).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      name: "Source",
      referenceName: "source",
      type: "shortText",
      required: false,
      position: 0,
    })
    const second = await api.post<Field>(`/v1/pipelines/${pipeline.id}/fields`, {
      name: "Budget",
      referenceName: "budget",
      type: "number",
      required: true,
    })
    expect(second.json.position).toBe(1)

    const listed = await api.get<{ items: Field[] }>(`/v1/pipelines/${pipeline.id}/fields`)
    expect(listed.status, listed.text).toBe(200)
    expect(listed.json).toEqual({ items: [created.json, second.json] })

    const updated = await api.patch<Field>(`/v1/pipelines/${pipeline.id}/fields/${second.json.id}`, {
      name: "Budget (USD)",
      required: false,
    })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).toEqual({ ...second.json, name: "Budget (USD)", required: false })

    expectValidation(
      await api.post(`/v1/pipelines/${pipeline.id}/fields`, { name: "Dup", referenceName: "source", type: "email" }),
      "A field with this reference name already exists",
    )
    expectValidation(
      await api.patch(`/v1/pipelines/${pipeline.id}/fields/${second.json.id}`, { referenceName: "source" }),
      "A field with this reference name already exists",
    )
    expectInvalid(await api.post(`/v1/pipelines/${pipeline.id}/fields`, { name: "x", referenceName: "x", type: "color" }), "type")
    expectInvalid(await api.post(`/v1/pipelines/${pipeline.id}/fields`, { name: "x", referenceName: "1x", type: "email" }), "referenceName")

    const deleted = await api.delete(`/v1/pipelines/${pipeline.id}/fields/${created.json.id}`)
    expect(deleted.status, deleted.text).toBe(204)
    expect((await api.get<{ items: Field[] }>(`/v1/pipelines/${pipeline.id}/fields`)).json.items).toEqual([updated.json])
    expectNotFound(await api.delete(`/v1/pipelines/${pipeline.id}/fields/${created.json.id}`), "Field not found")
    expectNotFound(await api.patch(`/v1/pipelines/${pipeline.id}/fields/${UNKNOWN_ID}`, { name: "x" }), "Field not found")
    expectNotFound(await api.get(`/v1/pipelines/${UNKNOWN_ID}/fields`), "Pipeline not found")
  })
})

describe("pipeline members", () => {
  it("setMember / listMembers / removeMember; edit rights imply view", async () => {
    const { api } = await newTenant("p113-mb")
    const pipeline = await create(api)
    const memberId = await ownerMemberId(api)
    expect((await api.get(`/v1/pipelines/${pipeline.id}/members`)).json).toEqual({ items: [] })

    const set = await api.put<Record<string, unknown>>(`/v1/pipelines/${pipeline.id}/members/${memberId}`, {
      canView: false,
      canEdit: true,
    })
    expect(set.status, set.text).toBe(200)
    expect(set.json).toEqual({
      workspaceMemberId: memberId,
      canView: true,
      canEdit: true,
      canDelete: false,
      canManageSettings: false,
    })
    const changed = await api.put(`/v1/pipelines/${pipeline.id}/members/${memberId}`, { canView: true, canManageSettings: true })
    expect(changed.json).toEqual({
      workspaceMemberId: memberId,
      canView: true,
      canEdit: false,
      canDelete: false,
      canManageSettings: true,
    })
    const listed = await api.get(`/v1/pipelines/${pipeline.id}/members`)
    expect(listed.status, listed.text).toBe(200)
    expect(listed.json).toEqual({ items: [changed.json] })

    const removed = await api.delete(`/v1/pipelines/${pipeline.id}/members/${memberId}`)
    expect(removed.status, removed.text).toBe(204)
    expect((await api.get(`/v1/pipelines/${pipeline.id}/members`)).json).toEqual({ items: [] })
    expectNotFound(await api.delete(`/v1/pipelines/${pipeline.id}/members/${memberId}`), "Pipeline member not found")
    expectNotFound(
      await api.put(`/v1/pipelines/${pipeline.id}/members/${UNKNOWN_ID}`, { canView: true }),
      "Workspace member not found",
    )
    expectInvalid(await api.put(`/v1/pipelines/${pipeline.id}/members/${memberId}`, {}), "canView")
  })

  it("a member of another workspace is not found", async () => {
    const { api } = await newTenant("p113-mb-foreign")
    const { api: other } = await newTenant("p113-mb-foreign-other")
    const pipeline = await create(api)
    const foreignMember = await ownerMemberId(other)
    expectNotFound(
      await api.put(`/v1/pipelines/${pipeline.id}/members/${foreignMember}`, { canView: true }),
      "Workspace member not found",
    )
  })
})

describe("pipelines.getStats", () => {
  it("counts created, won and lost transitions in the range by day and actor", async () => {
    const { api } = await newTenant("p113-stats")
    const pipeline = await create(api)
    const contactId = await newContact(api)
    const today = new Date().toISOString().slice(0, 10)
    const won = await api.post<{ id: string }>(`/v1/pipelines/${pipeline.id}/items`, { contactId, title: "Won", value: 100 })
    await api.post(`/v1/pipelines/${pipeline.id}/items`, { contactId, title: "Open", value: 7 })
    expect((await api.patch(`/v1/pipelines/${pipeline.id}/items/${won.json.id}`, { status: "won" })).status).toBe(200)

    const response = await api.get<Record<string, unknown>>(`/v1/pipelines/${pipeline.id}/stats`, {
      query: { from: today, to: today, timezone: "UTC" },
    })
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(["byActor", "change", "current", "daily", "previous", "range"])
    const current = { created: 2, won: 1, wonValue: 100, lost: 0, lostValue: 0 }
    const zero = { created: 0, won: 0, wonValue: 0, lost: 0, lostValue: 0 }
    expect(response.json).toEqual({
      range: { from: today, to: today, timezone: "UTC" },
      current,
      previous: zero,
      change: { created: null, won: null, wonValue: null, lost: 0, lostValue: 0 },
      daily: [{ day: today, ...current }],
      byActor: [{ actorKind: "automation", actorUserId: null, ...current }],
    })

    expectInvalid(await api.get(`/v1/pipelines/${pipeline.id}/stats`, { query: { from: "2026/01/01", to: today } }), "from")
    expectInvalid(await api.get(`/v1/pipelines/${pipeline.id}/stats`, { query: { from: today } }), "to")
    expectNotFound(await api.get(`/v1/pipelines/${UNKNOWN_ID}/stats`, { query: { from: today, to: today } }), "Pipeline not found")
    expectError(
      await api.get(`/v1/pipelines/${pipeline.id}/stats`, { query: { from: today, to: today, timezone: "Mars/Base" } }),
      422,
      "invalidRequestData",
    )
  })
})
