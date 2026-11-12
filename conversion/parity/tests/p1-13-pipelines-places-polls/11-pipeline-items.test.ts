// Pipeline items: createItem / getItem / listItems / stageTotals /
// updateItem / moveItem / transferItem / deleteItem / listContactItems,
// comments (listComments / addComment / deleteComment), attachments
// (addAttachment / removeAttachment, files from the media library upload)
// and listActivity. Item changes are pushed to the workspace realtime channel
// as `pipelineItemChanged`.

import { describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import {
  type Api,
  expectError,
  expectInvalid,
  expectNotFound,
  expectValidation,
  ISO,
  keysOf,
  memberSocket,
  newContact,
  newTenant,
  ownerMemberId,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueName,
  uploadMediaFile,
} from "./helpers.ts"

type Item = {
  id: string
  pipelineId: string
  stageId: string
  contactId: string
  contactName: string | null
  number: number
  title: string
  description: string | null
  value: number | null
  status: string
  priority: string
  dueDate: string | null
  fieldValues: Record<string, string>
  assigneeMemberIds: string[]
  assigneeInboxTeamIds: string[]
  commentCount: number
  createdById: string | null
  stageEnteredAt: string
  closedAt: string | null
  lastCommentAt: string | null
  createdAt: string
  updatedAt: string
}

const ITEM_KEYS = [
  "assigneeInboxTeamIds",
  "assigneeMemberIds",
  "closedAt",
  "commentCount",
  "contactId",
  "contactName",
  "createdAt",
  "createdById",
  "description",
  "dueDate",
  "fieldValues",
  "id",
  "lastCommentAt",
  "number",
  "pipelineId",
  "priority",
  "stageEnteredAt",
  "stageId",
  "status",
  "title",
  "updatedAt",
  "value",
]

type Board = { tenant: Tenant; api: Api; pipelineId: string; stageIds: string[]; contactId: string; firstName: string }

/** A pipeline with stages "Lead" + the given names, and one contact. */
const board = async (label: string, extraStages: string[] = ["Proposal"]): Promise<Board> => {
  const { tenant, api } = await newTenant(label)
  const created = await api.post<{ id: string; stages: Array<{ id: string }> }>("/v1/pipelines", {
    type: "opportunity",
    name: uniqueName("Sales"),
  })
  expect(created.status, created.text).toBe(201)
  const stageIds = [(created.json.stages[0] as { id: string }).id]
  for (const name of extraStages) {
    const stage = await api.post<{ id: string }>(`/v1/pipelines/${created.json.id}/stages`, { name })
    expect(stage.status, stage.text).toBe(201)
    stageIds.push(stage.json.id)
  }
  const firstName = uniqueName("Lea")
  const contactId = await newContact(api, firstName)
  return { tenant, api, pipelineId: created.json.id, stageIds, contactId, firstName }
}

const createItem = async (b: Board, body: Record<string, unknown> = {}): Promise<Item> => {
  const response = await b.api.post<Item>(`/v1/pipelines/${b.pipelineId}/items`, {
    contactId: b.contactId,
    title: uniqueName("Deal"),
    ...body,
  })
  expect(response.status, response.text).toBe(201)
  return response.json
}

const itemPath = (b: Board, itemId: string, suffix = "") => `/v1/pipelines/${b.pipelineId}/items/${itemId}${suffix}`

describe("pipelines.createItem / getItem", () => {
  it("creates an open medium item on the first stage, numbered per workspace", async () => {
    const b = await board("p113-it-create")
    const response = await b.api.post<Item>(`/v1/pipelines/${b.pipelineId}/items`, {
      contactId: b.contactId,
      title: "  First deal  ",
    })
    expect(response.status, response.text).toBe(201)
    const item = response.json
    expect(keysOf(item)).toEqual(ITEM_KEYS)
    expect(item).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      pipelineId: b.pipelineId,
      stageId: b.stageIds[0],
      contactId: b.contactId,
      contactName: expect.stringContaining(b.firstName),
      number: 1,
      title: "First deal",
      description: null,
      value: null,
      status: "open",
      priority: "medium",
      dueDate: null,
      fieldValues: {},
      assigneeMemberIds: [],
      assigneeInboxTeamIds: [],
      commentCount: 0,
      createdById: null,
      stageEnteredAt: expect.stringMatching(ISO),
      closedAt: null,
      lastCommentAt: null,
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
    })
    const detail = await b.api.get<Item & { attachments: unknown[] }>(itemPath(b, item.id))
    expect(detail.status, detail.text).toBe(200)
    expect(detail.json).toEqual({ ...item, attachments: [] })

    const memberId = await ownerMemberId(b.api)
    const full = await createItem(b, {
      title: "Full deal",
      description: "Big one",
      value: 1234.5,
      priority: "high",
      status: "won",
      dueDate: "2030-05-01T10:00:00.000Z",
      stageId: b.stageIds[1],
      assigneeMemberIds: [memberId],
    })
    expect(full).toMatchObject({
      number: 2,
      stageId: b.stageIds[1],
      description: "Big one",
      value: 1234.5,
      priority: "high",
      status: "won",
      dueDate: "2030-05-01T10:00:00.000Z",
      assigneeMemberIds: [memberId],
      closedAt: expect.stringMatching(ISO),
    })
  })

  it("stores custom field values and enforces required fields", async () => {
    const b = await board("p113-it-fields")
    const text = await b.api.post<{ id: string }>(`/v1/pipelines/${b.pipelineId}/fields`, {
      name: "Source",
      referenceName: "source",
      type: "shortText",
      required: true,
    })
    const number = await b.api.post<{ id: string }>(`/v1/pipelines/${b.pipelineId}/fields`, {
      name: "Seats",
      referenceName: "seats",
      type: "number",
    })
    const item = await createItem(b, { fieldValues: { [text.json.id]: "Referral" } })
    expect(item.fieldValues).toEqual({ [text.json.id]: "Referral" })
    const create = (fieldValues: Record<string, string | null>) =>
      b.api.post(`/v1/pipelines/${b.pipelineId}/items`, { contactId: b.contactId, title: "x", fieldValues })
    expectValidation(await create({}), "Source is required")
    expectValidation(await create({ [text.json.id]: "x", [number.json.id]: "many" }), "Invalid value for Seats")
    expectValidation(await create({ [text.json.id]: "x", [UNKNOWN_ID]: "1" }), `Unknown field ${UNKNOWN_ID}`)
  })

  it("an idempotency key returns the first item; reused for another contact -> 409", async () => {
    const b = await board("p113-it-idem")
    const key = uniqueName("idem")
    const first = await createItem(b, { idempotencyKey: key, title: "Once" })
    const replay = await createItem(b, { idempotencyKey: key, title: "Twice" })
    expect(replay.id).toBe(first.id)
    expect(replay.title).toBe("Once")
    const otherContact = await newContact(b.api)
    expectError(
      await b.api.post(`/v1/pipelines/${b.pipelineId}/items`, { contactId: otherContact, title: "x", idempotencyKey: key }),
      409,
      "pipelineItemIdempotencyKeyConflict",
      { message: "This idempotency key was already used for another contact" },
    )
  })

  it("refuses unknown contacts, stages, assignees and pipelines; caps open items per contact", async () => {
    const b = await board("p113-it-refused")
    const other = await board("p113-it-refused-other")
    const post = (body: Record<string, unknown>) =>
      b.api.post(`/v1/pipelines/${b.pipelineId}/items`, { contactId: b.contactId, title: "x", ...body })
    expectNotFound(await post({ contactId: UNKNOWN_ID }), "Contact not found")
    expectNotFound(await post({ contactId: other.contactId }), "Contact not found")
    expectNotFound(await post({ stageId: other.stageIds[0] }), "Stage not found")
    expectNotFound(await post({ assigneeMemberIds: [UNKNOWN_ID] }), "Assigned admin not found")
    expectNotFound(await post({ assigneeInboxTeamIds: [UNKNOWN_ID] }), "Assigned team not found")
    expectNotFound(
      await b.api.post(`/v1/pipelines/${other.pipelineId}/items`, { contactId: b.contactId, title: "x" }),
      "Pipeline not found",
    )
    expectInvalid(await post({ title: "" }), "title")
    expectInvalid(await post({ value: -1 }), "value")
    expectInvalid(await post({ priority: "urgent" }), "priority")
    expectInvalid(await post({ dueDate: "3000-01-01T00:00:00.000Z" }), "dueDate")

    expect((await b.api.patch(`/v1/pipelines/${b.pipelineId}`, { settings: { maxOpenItemsPerContact: 1 } })).status).toBe(200)
    await createItem(b)
    expectError(await post({}), 409, "pipelineItemLimitReached", {
      message: "This contact already has 1 open item(s) in this pipeline",
    })
    // A closed item does not count.
    expect((await post({ status: "lost" })).status).toBe(201)
    expectNotFound(await b.api.get(itemPath(b, UNKNOWN_ID)), "Item not found")
    expectNotFound(await other.api.get(`/v1/pipelines/${other.pipelineId}/items/${UNKNOWN_ID}`), "Item not found")
  })
})

describe("pipelines.listItems / stageTotals", () => {
  it("lists newest first with a cursor, sorts, filters by stage, status and search", async () => {
    const b = await board("p113-it-list")
    const items: Item[] = []
    for (let index = 0; index < 5; index += 1) {
      items.push(
        await createItem(b, {
          title: index === 3 ? "Special widget order" : `Deal ${index}`,
          value: (index + 1) * 10,
          stageId: b.stageIds[index % 2],
          status: index === 4 ? "won" : "open",
        }),
      )
    }
    const ids = items.map((item) => item.id)
    const page1 = await b.api.get<{ items: Item[]; nextCursor: string | null }>(`/v1/pipelines/${b.pipelineId}/items`, {
      query: { limit: 2 },
    })
    expect(page1.status, page1.text).toBe(200)
    expect(keysOf(page1.json)).toEqual(["items", "nextCursor"])
    expect(page1.json.items.map((item) => item.id)).toEqual([ids[4], ids[3]])
    expect(page1.json.items[1]).toEqual(items[3])
    expect(typeof page1.json.nextCursor).toBe("string")
    const page2 = await b.api.get<{ items: Item[]; nextCursor: string | null }>(`/v1/pipelines/${b.pipelineId}/items`, {
      query: { limit: 2, cursor: page1.json.nextCursor as string },
    })
    expect(page2.json.items.map((item) => item.id)).toEqual([ids[2], ids[1]])
    const page3 = await b.api.get<{ items: Item[]; nextCursor: string | null }>(`/v1/pipelines/${b.pipelineId}/items`, {
      query: { limit: 2, cursor: page2.json.nextCursor as string },
    })
    expect(page3.json).toEqual({ items: [items[0]], nextCursor: null })

    const byValue = await b.api.get<{ items: Item[] }>(`/v1/pipelines/${b.pipelineId}/items`, {
      query: { sort: "value", direction: "asc" },
    })
    expect(byValue.json.items.map((item) => item.value)).toEqual([10, 20, 30, 40, 50])
    const stage1 = await b.api.get<{ items: Item[] }>(`/v1/pipelines/${b.pipelineId}/items`, {
      query: { stageId: b.stageIds[1], sort: "createdAt", direction: "asc" },
    })
    expect(stage1.json.items.map((item) => item.id)).toEqual([ids[1], ids[3]])
    const won = await b.api.get<{ items: Item[] }>(`/v1/pipelines/${b.pipelineId}/items`, { query: { status: "won" } })
    expect(won.json.items.map((item) => item.id)).toEqual([ids[4]])
    const openOrWon = await b.api.get<{ items: Item[] }>(`/v1/pipelines/${b.pipelineId}/items`, {
      query: { status: "open,won" },
    })
    expect(openOrWon.json.items).toHaveLength(5)
    const searched = await b.api.get<{ items: Item[] }>(`/v1/pipelines/${b.pipelineId}/items`, { query: { search: "WIDGET" } })
    expect(searched.json.items.map((item) => item.id)).toEqual([ids[3]])
    const valueGt = await b.api.get<{ items: Item[] }>(`/v1/pipelines/${b.pipelineId}/items`, {
      query: { valueOperator: "gt", valueAmount: 30, sort: "value", direction: "asc" },
    })
    expect(valueGt.json.items.map((item) => item.value)).toEqual([40, 50])

    expectValidation(
      await b.api.get(`/v1/pipelines/${b.pipelineId}/items`, { query: { cursor: "not-a-cursor" } }),
      "Invalid cursor",
    )
    expectInvalid(await b.api.get(`/v1/pipelines/${b.pipelineId}/items`, { query: { valueOperator: "gt" } }), "valueAmount")
    expectInvalid(await b.api.get(`/v1/pipelines/${b.pipelineId}/items`, { query: { limit: 101 } }), "limit")
    expectInvalid(await b.api.get(`/v1/pipelines/${b.pipelineId}/items`, { query: { sort: "title" } }), "sort")
    expectNotFound(await b.api.get(`/v1/pipelines/${UNKNOWN_ID}/items`), "Pipeline not found")
  })

  it("stageTotals counts items and sums values per stage, with the same filters", async () => {
    const b = await board("p113-it-totals", ["Proposal", "Empty"])
    await createItem(b, { value: 10.5 })
    await createItem(b, { value: 4 })
    await createItem(b, { stageId: b.stageIds[1], value: 100, status: "lost" })
    const response = await b.api.get<{ stages: unknown[] }>(`/v1/pipelines/${b.pipelineId}/stage-totals`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      stages: [
        { stageId: b.stageIds[0], name: "Lead", count: 2, totalValue: 14.5 },
        { stageId: b.stageIds[1], name: "Proposal", count: 1, totalValue: 100 },
        { stageId: b.stageIds[2], name: "Empty", count: 0, totalValue: 0 },
      ],
    })
    const open = await b.api.get<{ stages: Array<{ count: number }> }>(`/v1/pipelines/${b.pipelineId}/stage-totals`, {
      query: { status: "open" },
    })
    expect(open.json.stages.map((stage) => stage.count)).toEqual([2, 0, 0])
    expectNotFound(await b.api.get(`/v1/pipelines/${UNKNOWN_ID}/stage-totals`), "Pipeline not found")
  })
})

describe("pipelines.updateItem / moveItem / transferItem / deleteItem", () => {
  it("updateItem changes only the sent fields; closing sets closedAt, reopening clears it", async () => {
    const b = await board("p113-it-update")
    const item = await createItem(b, { title: "Before", value: 5 })
    const updated = await b.api.patch<Item>(itemPath(b, item.id), { title: "After", priority: "low", status: "won" })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).toEqual({
      ...item,
      title: "After",
      priority: "low",
      status: "won",
      closedAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
    })
    const reopened = await b.api.patch<Item>(itemPath(b, item.id), { status: "open", value: null })
    expect(reopened.json).toMatchObject({ status: "open", closedAt: null, value: null, title: "After" })
    const moved = await b.api.patch<Item>(itemPath(b, item.id), { stageId: b.stageIds[1] })
    expect(moved.json.stageId).toBe(b.stageIds[1])
    expectNotFound(await b.api.patch(itemPath(b, UNKNOWN_ID), { title: "x" }), "Item not found")
  })

  it("moveItem changes the stage and its entry time; 404 for a stage of another pipeline", async () => {
    const b = await board("p113-it-move")
    const other = await board("p113-it-move-other")
    const item = await createItem(b)
    const response = await b.api.post<Item>(itemPath(b, item.id, "/move"), { stageId: b.stageIds[1] })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ id: item.id, stageId: b.stageIds[1] })
    expect(Date.parse(response.json.stageEnteredAt)).toBeGreaterThanOrEqual(Date.parse(item.stageEnteredAt))
    expectNotFound(await b.api.post(itemPath(b, item.id, "/move"), { stageId: other.stageIds[0] }), "Stage not found")
    expectNotFound(await b.api.post(itemPath(b, UNKNOWN_ID, "/move"), { stageId: b.stageIds[0] }), "Item not found")
    expectInvalid(await b.api.post(itemPath(b, item.id, "/move"), {}), "stageId")
  })

  it("transferItem moves the item to another pipeline's first stage, keeping fields by reference name", async () => {
    const b = await board("p113-it-transfer")
    const source = await b.api.post<{ id: string }>(`/v1/pipelines/${b.pipelineId}/fields`, {
      name: "Region",
      referenceName: "region",
      type: "shortText",
    })
    const destination = await b.api.post<{ id: string; stages: Array<{ id: string }> }>("/v1/pipelines", {
      type: "ticket",
      name: uniqueName("Support"),
    })
    const destinationField = await b.api.post<{ id: string }>(`/v1/pipelines/${destination.json.id}/fields`, {
      name: "Region",
      referenceName: "region",
      type: "shortText",
    })
    const item = await createItem(b, { fieldValues: { [source.json.id]: "EMEA" } })
    const response = await b.api.post<Item>(itemPath(b, item.id, "/transfer"), { destinationPipelineId: destination.json.id })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({
      id: item.id,
      pipelineId: destination.json.id,
      stageId: destination.json.stages[0]?.id,
      number: item.number,
      fieldValues: { [destinationField.json.id]: "EMEA" },
    })
    expectNotFound(await b.api.get(itemPath(b, item.id)), "Item not found")
    const moved = await b.api.get<Item>(`/v1/pipelines/${destination.json.id}/items/${item.id}`)
    expect(moved.json.pipelineId).toBe(destination.json.id)
    expectValidation(
      await b.api.post(`/v1/pipelines/${destination.json.id}/items/${item.id}/transfer`, {
        destinationPipelineId: destination.json.id,
      }),
      "Choose a different pipeline",
    )
    expectNotFound(
      await b.api.post(`/v1/pipelines/${destination.json.id}/items/${item.id}/transfer`, { destinationPipelineId: UNKNOWN_ID }),
      "Pipeline not found",
    )
  })

  it("deleteItem is 204; then the item is gone", async () => {
    const b = await board("p113-it-delete")
    const item = await createItem(b)
    const response = await b.api.delete(itemPath(b, item.id))
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectNotFound(await b.api.get(itemPath(b, item.id)), "Item not found")
    expectNotFound(await b.api.delete(itemPath(b, item.id)), "Item not found")
  })
})

describe("pipelines.listContactItems", () => {
  it("lists the contact's items across pipelines, newest first, with the pipeline name and type", async () => {
    const b = await board("p113-it-contact")
    const second = await b.api.post<{ id: string; name: string }>("/v1/pipelines", { type: "custom", name: uniqueName("Ops") })
    const first = await createItem(b)
    const other = await b.api.post<Item>(`/v1/pipelines/${second.json.id}/items`, { contactId: b.contactId, title: "Ops task" })
    expect(other.status, other.text).toBe(201)
    await createItem(b, { contactId: await newContact(b.api) })
    const response = await b.api.get<{ items: Array<Item & { pipelineName: string; pipelineType: string }> }>(
      `/v1/contacts/id:${b.contactId}/pipeline-items`,
    )
    expect(response.status, response.text).toBe(200)
    expect(response.json.items.map((item) => [item.id, item.pipelineType])).toEqual([
      [other.json.id, "custom"],
      [first.id, "opportunity"],
    ])
    expect(response.json.items[0]).toEqual({ ...other.json, pipelineName: second.json.name, pipelineType: "custom" })
    expectNotFound(await b.api.get(`/v1/contacts/id:${UNKNOWN_ID}/pipeline-items`), "Contact not found")
    // A bare id (no `id:` prefix) is a 404 in the original.
    expectNotFound(await b.api.get(`/v1/contacts/${b.contactId}/pipeline-items`), "Invalid identifier format")
  })
})

describe("pipeline item comments", () => {
  it("addComment / listComments (newest first) / deleteComment keep commentCount and lastCommentAt", async () => {
    const b = await board("p113-it-comments")
    const item = await createItem(b)
    const first = await b.api.post<Record<string, unknown>>(itemPath(b, item.id, "/comments"), { text: "  First note  " })
    expect(first.status, first.text).toBe(201)
    expect(first.json).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      itemId: item.id,
      text: "First note",
      createdById: null,
      createdAt: expect.stringMatching(ISO),
    })
    const key = uniqueName("c")
    const second = await b.api.post<{ id: string }>(itemPath(b, item.id, "/comments"), { text: "Second", idempotencyKey: key })
    const replay = await b.api.post<{ id: string }>(itemPath(b, item.id, "/comments"), { text: "Again", idempotencyKey: key })
    expect(replay.json.id).toBe(second.json.id)

    const listed = await b.api.get<{ items: Array<{ id: string }>; nextCursor: string | null }>(itemPath(b, item.id, "/comments"))
    expect(listed.status, listed.text).toBe(200)
    expect(listed.json.items.map((comment) => comment.id)).toEqual([second.json.id, first.json.id as string])
    expect(listed.json.nextCursor).toBeNull()
    const withComments = await b.api.get<Item>(itemPath(b, item.id))
    expect(withComments.json).toMatchObject({ commentCount: 2, lastCommentAt: expect.stringMatching(ISO) })

    const deleted = await b.api.delete(itemPath(b, item.id, `/comments/${second.json.id}`))
    expect(deleted.status, deleted.text).toBe(204)
    expect((await b.api.get<Item>(itemPath(b, item.id))).json).toMatchObject({
      commentCount: 1,
      lastCommentAt: first.json.createdAt,
    })
    expectNotFound(await b.api.delete(itemPath(b, item.id, `/comments/${second.json.id}`)), "Comment not found")
    expectInvalid(await b.api.post(itemPath(b, item.id, "/comments"), { text: "   " }), "text")
    expectNotFound(await b.api.post(itemPath(b, UNKNOWN_ID, "/comments"), { text: "x" }), "Item not found")
    expectNotFound(await b.api.get(itemPath(b, UNKNOWN_ID, "/comments")), "Item not found")
  })
})

describe("pipeline item attachments", () => {
  it("addAttachment links a media library file; removeAttachment only detaches it", async () => {
    const b = await board("p113-it-attach")
    const item = await createItem(b)
    const file = await uploadMediaFile(b.api)
    const added = await b.api.post(itemPath(b, item.id, "/attachments"), { mediaLibraryFileId: file.id })
    expect(added.status, added.text).toBe(204)
    const detail = await b.api.get<{ attachments: unknown[] }>(itemPath(b, item.id))
    expect(detail.json.attachments).toEqual([
      { mediaLibraryFileId: file.id, name: file.name, mimeType: "image/png", size: file.size, path: file.path },
    ])
    // Adding it again changes nothing.
    expect((await b.api.post(itemPath(b, item.id, "/attachments"), { mediaLibraryFileId: file.id })).status).toBe(204)
    expect((await b.api.get<{ attachments: unknown[] }>(itemPath(b, item.id))).json.attachments).toHaveLength(1)

    const removed = await b.api.delete(itemPath(b, item.id, `/attachments/${file.id}`))
    expect(removed.status, removed.text).toBe(204)
    expect((await b.api.get<{ attachments: unknown[] }>(itemPath(b, item.id))).json.attachments).toEqual([])
    expect((await b.api.get(`/v1/media-library/files/${file.id}`)).status).toBe(200)
    expectNotFound(await b.api.delete(itemPath(b, item.id, `/attachments/${file.id}`)), "Attachment not found")
    expectNotFound(
      await b.api.post(itemPath(b, item.id, "/attachments"), { mediaLibraryFileId: UNKNOWN_ID }),
      "Attachment file not found",
    )
    // Attachments can also come with the create.
    const withFile = await createItem(b, { mediaLibraryFileIds: [file.id] })
    expect((await b.api.get<{ attachments: unknown[] }>(itemPath(b, withFile.id))).json.attachments).toHaveLength(1)
  })
})

describe("pipelines.listActivity", () => {
  it("lists the item's history newest first, ten per page, filterable by group", async () => {
    const b = await board("p113-it-activity")
    const item = await createItem(b, { title: "Tracked", value: 1 })
    expect((await b.api.post(itemPath(b, item.id, "/move"), { stageId: b.stageIds[1] })).status).toBe(200)
    expect((await b.api.patch(itemPath(b, item.id), { status: "won" })).status).toBe(200)
    expect((await b.api.post(itemPath(b, item.id, "/comments"), { text: "Note" })).status).toBe(201)
    for (let value = 2; value <= 9; value += 1) {
      expect((await b.api.patch(itemPath(b, item.id), { value })).status).toBe(200)
    }

    type Activity = Record<string, unknown> & { kind: string }
    const page1 = await b.api.get<{ items: Activity[]; nextCursor: string | null }>(itemPath(b, item.id, "/activity"))
    expect(page1.status, page1.text).toBe(200)
    expect(page1.json.items).toHaveLength(10)
    expect(page1.json.items[0]).toMatchObject({ kind: "valueChanged", fromValue: 8, toValue: 9, actorUserId: null })
    expect(typeof page1.json.nextCursor).toBe("string")
    const page2 = await b.api.get<{ items: Activity[]; nextCursor: string | null }>(itemPath(b, item.id, "/activity"), {
      query: { cursor: page1.json.nextCursor as string },
    })
    // Newest first: the 8 value changes, the comment and the status change fill page 1.
    expect(page1.json.items.slice(8).map((activity) => activity.kind)).toEqual(["commentAdded", "statusChanged"])
    expect(page1.json.items[9]).toMatchObject({ kind: "statusChanged", fromStatus: "open", toStatus: "won" })
    expect(page2.json.items.map((activity) => activity.kind)).toEqual(["stageChanged", "created"])
    expect(page2.json.nextCursor).toBeNull()
    expect(page2.json.items[0]).toMatchObject({
      kind: "stageChanged",
      fromStageId: b.stageIds[0],
      toStageId: b.stageIds[1],
      fromStageName: "Lead",
      toStageName: "Proposal",
    })
    expect(keysOf(page2.json.items[1])).toEqual(
      [
        "actorKind",
        "actorUserId",
        "fromDueDate",
        "fromPipelineId",
        "fromPipelineName",
        "fromPriority",
        "fromStageId",
        "fromStageName",
        "fromStatus",
        "fromTitle",
        "fromValue",
        "id",
        "kind",
        "occurredAt",
        "pipelineName",
        "toDueDate",
        "toPriority",
        "toStageId",
        "toStageName",
        "toStatus",
        "toTitle",
        "toValue",
      ].sort(),
    )

    const stageOnly = await b.api.get<{ items: Activity[] }>(itemPath(b, item.id, "/activity"), { query: { groups: "stage,status" } })
    expect(stageOnly.json.items.map((activity) => activity.kind)).toEqual(["statusChanged", "stageChanged"])
    expectValidation(await b.api.get(itemPath(b, item.id, "/activity"), { query: { cursor: "garbage" } }), "Invalid cursor")
    expectInvalid(await b.api.get(itemPath(b, item.id, "/activity"), { query: { groups: "nope" } }), "groups.0")
    expectNotFound(await b.api.get(itemPath(b, UNKNOWN_ID, "/activity")), "Item not found")
  })
})

describe("pipeline item realtime", () => {
  it("pushes pipelineItemChanged to the workspace on create, move and delete", async () => {
    const b = await board("p113-it-realtime")
    const socket = await memberSocket(b.tenant)
    try {
      const item = await createItem(b)
      const created = await socket.waitFor(
        "pipelineItemChanged",
        (event) => (event.data as { itemId?: string; change?: string }).itemId === item.id,
      )
      expect(created.data).toEqual({ pipelineId: b.pipelineId, itemId: item.id, contactId: b.contactId, change: "created" })
      expect((await b.api.post(itemPath(b, item.id, "/move"), { stageId: b.stageIds[1] })).status).toBe(200)
      const moved = await socket.waitFor(
        "pipelineItemChanged",
        (event) => (event.data as { change?: string }).change === "moved",
      )
      expect(moved.data).toMatchObject({ pipelineId: b.pipelineId, itemId: item.id, change: "moved" })
      expect((await b.api.delete(itemPath(b, item.id))).status).toBe(204)
      const deleted = await socket.waitFor(
        "pipelineItemChanged",
        (event) => (event.data as { change?: string }).change === "deleted",
      )
      expect(deleted.data).toMatchObject({ pipelineId: b.pipelineId, itemId: item.id, change: "deleted" })
    } finally {
      await socket.close()
    }
  })
})
