// keywords.* (keyword automations, `AutomatedResponse` rows): create, get,
// list, update (PATCH), updateStatus (PATCH .../status), delete. Every route
// takes `type` (inbound = contact messages, the default; outbound = Page/agent
// messages). Inbound rules always store `conditions` (plain keywords become one
// `contains` condition) and mirror their trigger words in `keywords`.

import { describe, expect, it } from "vitest"
import {
  type Api,
  createPublishedFlow,
  expectError,
  expectInvalid,
  expectNotFound,
  expectValidation,
  ISO,
  issuePaths,
  newTenant,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueName,
} from "./helpers.ts"

type Keyword = {
  id: string
  keywords: string[]
  conditions: Array<{ operator: string; values: string[] }> | null
  status: boolean
  text: string | null
  flowId: string | null
  type: "inbound" | "outbound"
  createdAt: string
  updatedAt: string
}

const KEYS = ["conditions", "createdAt", "flowId", "id", "keywords", "status", "text", "type", "updatedAt"]
const NOT_FOUND = "Automated response not found"

const create = async (api: Api, body: Record<string, unknown>): Promise<Keyword> => {
  const response = await api.post<Keyword>("/v1/keywords", body)
  expect(response.status, response.text).toBe(201)
  return response.json
}

const get = async (api: Api, id: string, type?: string): Promise<Keyword> => {
  const response = await api.get<Keyword>(`/v1/keywords/${id}`, { query: { type } })
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("keywords.create", () => {
  it("inbound keywords become one trimmed `contains` condition; the reply text is stored (201)", async () => {
    const { api } = await newTenant("p110-kw-create")
    const response = await api.post<Keyword>("/v1/keywords", { keywords: ["Hello", " price ", "price"], text: "Hi there" })
    expect(response.status, response.text).toBe(201)
    expect(Object.keys(response.json).sort()).toEqual(KEYS)
    expect(response.json).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      keywords: ["Hello", "price"],
      conditions: [{ operator: "contains", values: ["Hello", "price", "price"] }],
      status: true,
      text: "Hi there",
      flowId: null,
      type: "inbound",
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
    })
    expect(await get(api, response.json.id)).toEqual(response.json)
  })

  it("inbound conditions are stored as sent; `keywords` mirrors only trigger words", async () => {
    const { api } = await newTenant("p110-kw-conditions")
    const conditions = [
      { operator: "is", values: ["Exact"] },
      { operator: "notContains", values: ["spam"] },
      { operator: "matchesRegex", values: ["^order\\s+\\d+$"] },
      { operator: "beginsWith", values: [" Hey "] },
      { operator: "containsPhone", values: [] },
    ]
    const keyword = await create(api, { conditions, text: "ok" })
    expect(keyword.keywords).toEqual(["Exact", "Hey"])
    expect(keyword.conditions).toEqual([
      { operator: "is", values: ["Exact"] },
      { operator: "notContains", values: ["spam"] },
      { operator: "matchesRegex", values: ["^order\\s+\\d+$"] },
      { operator: "beginsWith", values: ["Hey"] },
      { operator: "containsPhone", values: [] },
    ])
  })

  it("an outbound rule keeps exact keywords and no conditions; a flow reply has no text", async () => {
    const { api } = await newTenant("p110-kw-outbound")
    const flowId = await createPublishedFlow(api, uniqueName("Reply"), "From flow")
    const keyword = await create(api, { type: "outbound", keywords: [" thanks "], flowId })
    expect(keyword).toMatchObject({ type: "outbound", keywords: ["thanks"], conditions: null, text: null, flowId, status: true })
    expect(await get(api, keyword.id, "outbound")).toEqual(keyword)
  })

  it("refuses text and flow together, an unknown flow or folder, and blank keywords", async () => {
    const { api } = await newTenant("p110-kw-create-biz")
    const flowId = await createPublishedFlow(api, uniqueName("Reply"), "x")
    expectValidation(
      await api.post("/v1/keywords", { keywords: ["a"], text: "t", flowId }),
      "A keyword replies with either text or a flow, not both",
    )
    // create checks the flow inside the template lock (404, not declared on the route);
    // update checks it with a 422 validation error instead.
    expectError(await api.post("/v1/keywords", { keywords: ["a"], flowId: UNKNOWN_ID }), 404, "notFound", {
      defined: false,
      message: "Flow not found",
    })
    expectError(await api.post("/v1/keywords", { keywords: ["a"], text: "t", folderId: UNKNOWN_ID }), 404, "notFound", {
      defined: false,
      message: "Folder not found",
    })
    expectValidation(await api.post("/v1/keywords", { keywords: ["   "], text: "t" }), "At least one keyword is required")
    expectValidation(
      await api.post("/v1/keywords", { type: "outbound", conditions: [{ operator: "is", values: ["x"] }], text: "t" }),
      "Conditions are only supported for contact (inbound) keywords",
    )
    expect((await api.get<{ data: unknown[] }>("/v1/keywords")).json.data).toEqual([])
  })

  it("validates the body (422 invalidRequestData)", async () => {
    const { api } = await newTenant("p110-kw-create-invalid")
    expectInvalid(await api.post("/v1/keywords", { text: "t" }), "keywords")
    expectInvalid(
      await api.post("/v1/keywords", { keywords: ["a"], conditions: [{ operator: "is", values: ["a"] }], text: "t" }),
      "conditions",
    )
    expectInvalid(await api.post("/v1/keywords", { keywords: [], text: "t" }), "keywords")
    expectInvalid(await api.post("/v1/keywords", { keywords: [""], text: "t" }), "keywords.0")
    expectInvalid(await api.post("/v1/keywords", { keywords: ["x".repeat(256)], text: "t" }), "keywords.0")
    expectInvalid(await api.post("/v1/keywords", { keywords: ["a"], text: "" }), "text")
    expectInvalid(await api.post("/v1/keywords", { keywords: ["a"], type: "both" }), "type")
    expectInvalid(await api.post("/v1/keywords", { keywords: ["a"], flowId: "abc" }), "flowId")
    expectInvalid(
      await api.post("/v1/keywords", { conditions: [{ operator: "containsPhone", values: ["x"] }], text: "t" }),
      "conditions.0.values",
    )
    expectInvalid(
      await api.post("/v1/keywords", { conditions: [{ operator: "contains", values: [] }], text: "t" }),
      "conditions.0.values",
    )
    expectInvalid(
      await api.post("/v1/keywords", { conditions: [{ operator: "matchesRegex", values: ["("] }], text: "t" }),
      "conditions.0.values.0",
    )
    expectInvalid(
      await api.post("/v1/keywords", { conditions: [{ operator: "startsWith", values: ["a"] }], text: "t" }),
      "conditions.0.operator",
    )
    expectInvalid(await api.post("/v1/keywords", { conditions: [], text: "t" }), "conditions")
    expectInvalid(
      await api.post("/v1/keywords", {
        conditions: Array.from({ length: 11 }, (_, index) => ({ operator: "contains", values: [`k${index}`] })),
        text: "t",
      }),
      "conditions",
    )
  })
})

describe("keywords.get / list", () => {
  it("get needs the rule's type; 404 for unknown, foreign or other-type ids", async () => {
    const { api } = await newTenant("p110-kw-get")
    const outbound = await create(api, { type: "outbound", keywords: ["bye"], text: "x" })
    expectNotFound(await api.get(`/v1/keywords/${outbound.id}`), NOT_FOUND)
    expect((await get(api, outbound.id, "outbound")).id).toBe(outbound.id)
    expectNotFound(await api.get(`/v1/keywords/${UNKNOWN_ID}`), NOT_FOUND)
    const { api: otherApi } = await newTenant("p110-kw-get-o")
    expectNotFound(await otherApi.get(`/v1/keywords/${outbound.id}`, { query: { type: "outbound" } }), NOT_FOUND)
    expectInvalid(await api.get("/v1/keywords/abc"), "id")
    expectInvalid(await api.get(`/v1/keywords/${outbound.id}`, { query: { type: "x" } }), "type")
  })

  it("lists one direction, newest first, paged", async () => {
    const { api } = await newTenant("p110-kw-list")
    const a = await create(api, { keywords: ["a"], text: "1" })
    const b = await create(api, { keywords: ["b"], text: "2" })
    const c = await create(api, { keywords: ["c"], text: "3" })
    const out = await create(api, { type: "outbound", keywords: ["o"], text: "4" })

    const inbound = await api.get<{ data: Keyword[]; pageCount: number }>("/v1/keywords")
    expect(inbound.status, inbound.text).toBe(200)
    expect(inbound.json.pageCount).toBe(1)
    expect(inbound.json.data.map((item) => item.id)).toEqual([c.id, b.id, a.id])
    expect(inbound.json.data[0]).toEqual(c)

    const outbound = await api.get<{ data: Keyword[]; pageCount: number }>("/v1/keywords", { query: { type: "outbound" } })
    expect(outbound.json).toEqual({ data: [out], pageCount: 1 })

    const page2 = await api.get<{ data: Keyword[]; pageCount: number }>("/v1/keywords", { query: { page: 2, perPage: 2 } })
    expect(page2.json.pageCount).toBe(2)
    expect(page2.json.data.map((item) => item.id)).toEqual([a.id])

    const { api: otherApi } = await newTenant("p110-kw-list-o")
    expect((await otherApi.get("/v1/keywords")).json).toEqual({ data: [], pageCount: 0 })
    expectInvalid(await api.get("/v1/keywords", { query: { perPage: 51 } }), "perPage")
    expectInvalid(await api.get("/v1/keywords", { query: { page: 0 } }), "page")
  })
})

describe("keywords.update", () => {
  it("text replaces a flow reply and a flow replaces a text reply; keywords replace the conditions", async () => {
    const { api } = await newTenant("p110-kw-update")
    const flowId = await createPublishedFlow(api, uniqueName("Reply"), "x")
    const keyword = await create(api, { conditions: [{ operator: "is", values: ["exact"] }], text: "first" })

    const toFlow = await api.patch<Keyword>(`/v1/keywords/${keyword.id}`, { flowId })
    expect(toFlow.status, toFlow.text).toBe(200)
    expect(toFlow.json).toMatchObject({ id: keyword.id, flowId, text: null, conditions: keyword.conditions })

    const toText = await api.patch<Keyword>(`/v1/keywords/${keyword.id}`, { text: "second" })
    expect(toText.json).toMatchObject({ flowId: null, text: "second" })

    // Text wins when both are sent.
    const both = await api.patch<Keyword>(`/v1/keywords/${keyword.id}`, { text: "third", flowId })
    expect(both.status, both.text).toBe(200)
    expect(both.json).toMatchObject({ flowId: null, text: "third" })

    const words = await api.patch<Keyword>(`/v1/keywords/${keyword.id}`, { keywords: [" new ", "word"] })
    expect(words.json).toMatchObject({
      keywords: ["new", "word"],
      conditions: [{ operator: "contains", values: ["new", "word"] }],
      text: "third",
      status: true,
    })
    expect(await get(api, keyword.id)).toEqual(words.json)
  })

  it("an outbound rule is updated with type=outbound only", async () => {
    const { api } = await newTenant("p110-kw-update-out")
    const keyword = await create(api, { type: "outbound", keywords: ["bye"], text: "x" })
    expectNotFound(await api.patch(`/v1/keywords/${keyword.id}`, { text: "y" }), NOT_FOUND)
    const updated = await api.patch<Keyword>(`/v1/keywords/${keyword.id}`, { type: "outbound", keywords: ["ciao"] })
    expect(updated.json).toMatchObject({ keywords: ["ciao"], conditions: null, text: "x" })
    expectValidation(
      await api.patch(`/v1/keywords/${keyword.id}`, { type: "outbound", conditions: [{ operator: "is", values: ["a"] }] }),
      "Conditions are only supported for contact (inbound) keywords",
    )
  })

  it("refuses an empty change, an unknown flow, unknown ids and bad input", async () => {
    const { api } = await newTenant("p110-kw-update-bad")
    const keyword = await create(api, { keywords: ["a"], text: "x" })
    const empty = await api.patch(`/v1/keywords/${keyword.id}`, {})
    expectError(empty, 422, "invalidRequestData", { defined: true })
    expect(issuePaths(empty)).toEqual([""])
    expectError(await api.patch(`/v1/keywords/${keyword.id}`, { status: false }), 422, "invalidRequestData")
    expectValidation(await api.patch(`/v1/keywords/${keyword.id}`, { flowId: UNKNOWN_ID }), "Flow not found")
    expectNotFound(await api.patch(`/v1/keywords/${UNKNOWN_ID}`, { text: "y" }), NOT_FOUND)
    expectInvalid(await api.patch(`/v1/keywords/${keyword.id}`, { keywords: ["a"], conditions: [{ operator: "is", values: ["a"] }] }), "conditions")
    expectInvalid(await api.patch(`/v1/keywords/${keyword.id}`, { text: "" }), "text")
    expect(await get(api, keyword.id)).toEqual(keyword)
  })
})

describe("keywords.updateStatus", () => {
  it("disables and enables a rule without touching the rest", async () => {
    const { api } = await newTenant("p110-kw-status")
    const keyword = await create(api, { keywords: ["a"], text: "x" })
    const off = await api.patch<Keyword>(`/v1/keywords/${keyword.id}/status`, { status: false })
    expect(off.status, off.text).toBe(200)
    expect(off.json).toEqual({ ...keyword, status: false, updatedAt: expect.stringMatching(ISO) })
    expect((await get(api, keyword.id)).status).toBe(false)
    const on = await api.patch<Keyword>(`/v1/keywords/${keyword.id}/status`, { status: true })
    expect(on.json.status).toBe(true)
  })

  it("404 for unknown or other-type ids; 422 without status", async () => {
    const { api } = await newTenant("p110-kw-status-bad")
    const outbound = await create(api, { type: "outbound", keywords: ["a"], text: "x" })
    expectNotFound(await api.patch(`/v1/keywords/${outbound.id}/status`, { status: false }), NOT_FOUND)
    expect((await api.patch(`/v1/keywords/${outbound.id}/status`, { status: false, type: "outbound" })).status).toBe(200)
    expectNotFound(await api.patch(`/v1/keywords/${UNKNOWN_ID}/status`, { status: true }), NOT_FOUND)
    expectInvalid(await api.patch(`/v1/keywords/${outbound.id}/status`, {}), "status")
    expectInvalid(await api.patch(`/v1/keywords/${outbound.id}/status`, { status: "no" }), "status")
    // oRPC coerces "off" to false.
    const coerced = await api.patch<Keyword>(`/v1/keywords/${outbound.id}/status`, { status: "on", type: "outbound" })
    expect(coerced.status, coerced.text).toBe(200)
    expect(coerced.json.status).toBe(true)
  })
})

describe("keywords.delete", () => {
  it("deletes a rule (204); unknown ids and other-type ids are a silent 204", async () => {
    const { api } = await newTenant("p110-kw-delete")
    const inbound = await create(api, { keywords: ["a"], text: "x" })
    const outbound = await create(api, { type: "outbound", keywords: ["b"], text: "y" })
    const response = await api.delete(`/v1/keywords/${inbound.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectNotFound(await api.get(`/v1/keywords/${inbound.id}`), NOT_FOUND)
    expect((await api.delete(`/v1/keywords/${inbound.id}`)).status).toBe(204)
    expect((await api.delete(`/v1/keywords/${UNKNOWN_ID}`)).status).toBe(204)
    // Wrong type: nothing is deleted.
    expect((await api.delete(`/v1/keywords/${outbound.id}`)).status).toBe(204)
    expect((await get(api, outbound.id, "outbound")).id).toBe(outbound.id)
    // DELETE reads `type` from the body, not from the query string.
    expect((await api.delete(`/v1/keywords/${outbound.id}`, { query: { type: "outbound" } })).status).toBe(204)
    expect((await get(api, outbound.id, "outbound")).id).toBe(outbound.id)
    expect((await api.delete(`/v1/keywords/${outbound.id}`, { json: { type: "outbound" } })).status).toBe(204)
    expectNotFound(await api.get(`/v1/keywords/${outbound.id}`, { query: { type: "outbound" } }), NOT_FOUND)
    expectInvalid(await api.delete("/v1/keywords/abc"), "id")
  })

  it("does not delete another workspace's rule", async () => {
    const { api } = await newTenant("p110-kw-delete-iso")
    const keyword = await create(api, { keywords: ["a"], text: "x" })
    const { api: otherApi } = await newTenant("p110-kw-delete-iso-o")
    expect((await otherApi.delete(`/v1/keywords/${keyword.id}`)).status).toBe(204)
    expect((await get(api, keyword.id)).id).toBe(keyword.id)
  })
})
