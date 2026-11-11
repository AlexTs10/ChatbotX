// contacts.listFilterFields / resolveFilterValueLabels.

import { beforeAll, describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { expectNormalized, Normalizer } from "../../src/normalize.ts"
import { type Api, createCustomField, createTag, expectInvalid, newTenant, qs } from "./helpers.ts"

type StaticField = { field: string; schemaKind: string; optionSource: string; operators: string[]; valueSource?: unknown }
type FilterFields = {
  staticFields: StaticField[]
  customFields: Array<{ id: string; name: string; type: string; valueType: string }>
  botFields: Array<{ id: string; name: string; type: string; valueType: string }>
  tags: Array<{ id: string; name: string }>
  pipelineItems: { pipelines: unknown[]; attributes: unknown[] }
}

let tenant: Tenant
let api: Api

beforeAll(async () => {
  ;({ tenant, api } = await newTenant("p106-ff"))
})

describe("listFilterFields", () => {
  it("static fields with operators, plus the workspace's custom fields, bot fields and tags", async () => {
    const empty = await api.get<FilterFields>("/v1/contacts/filter-fields")
    expect(empty.status, empty.text).toBe(200)
    expect(Object.keys(empty.json).sort()).toEqual(["botFields", "customFields", "pipelineItems", "staticFields", "tags"])
    expect(empty.json).toMatchObject({ customFields: [], botFields: [], tags: [] })
    const fields = empty.json.staticFields.map((row) => row.field)
    expect(fields).toEqual(expect.arrayContaining(["fullName", "email", "phone", "tags", "contactCreatedAt", "currentChannel"]))
    expect(fields).not.toContain("isWithinWorkingHours")
    expect(empty.json.staticFields.find((row) => row.field === "gender")).toEqual({
      field: "gender",
      schemaKind: "select",
      optionSource: "gender",
      operators: ["eq", "ne", "isEmpty"],
    })
    // The whole static catalog, stable per build.
    expect(empty.json.staticFields).toMatchSnapshot()
    expect(empty.json.pipelineItems).toMatchSnapshot()
  })

  it("lists workspace entities with each custom / bot field's valueType", async () => {
    const n = new Normalizer()
    const { tenant: t, api: a } = await newTenant("p106-ff-entities")
    const tagId = await createTag(a, "ff-tag")
    const numberField = await createCustomField(a, "ff_number", "number")
    const dateField = await createCustomField(a, "ff_date", "date")
    const textField = await createCustomField(a, "ff_text", "longText")
    const bot = await a.post<{ id: string }>("/v1/bot-fields", { name: "ff_bot", type: "boolean", value: "true" })
    expect(bot.status, bot.text).toBe(201)
    const response = await a.get<FilterFields>("/v1/contacts/filter-fields")
    expect(response.status, response.text).toBe(200)
    expect(response.json.tags).toEqual([{ id: tagId, name: "ff-tag" }])
    expect([...response.json.customFields].sort((x, y) => x.name.localeCompare(y.name))).toEqual([
      { id: dateField, name: "ff_date", type: "date", valueType: "datetime" },
      { id: numberField, name: "ff_number", type: "number", valueType: "number" },
      { id: textField, name: "ff_text", type: "longText", valueType: "text" },
    ])
    expect(response.json.botFields).toEqual([{ id: bot.json.id, name: "ff_bot", type: "boolean", valueType: "boolean" }])
    expectNormalized(n, { ...response.json, staticFields: "<static>" }, { replace: { [t.workspaceId]: "<workspace>" } }).toMatchSnapshot()
  })

  it("surface=flowCondition adds the runtime-only fields and drops pipeline items", async () => {
    const response = await api.get<FilterFields>(`/v1/contacts/filter-fields${qs({ surface: "flowCondition" })}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json.staticFields.map((row) => row.field)).toContain("isWithinWorkingHours")
    expect(response.json.pipelineItems).toMatchSnapshot()
    expect((await api.get(`/v1/contacts/filter-fields${qs({ surface: "contacts" })}`)).status).toBe(200)
    expectInvalid(await api.get(`/v1/contacts/filter-fields${qs({ surface: "broadcasts" })}`), "surface")
  })
})

describe("resolveFilterValueLabels", () => {
  it("returns names of the referenced tags; unknown ids are absent", async () => {
    const tagId = await createTag(api, "label-tag")
    const response = await api.post("/v1/contacts/filter-value-labels", { tags: [tagId, "999"] })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ tags: [{ id: tagId, name: "label-tag" }] })
    expect((await api.post("/v1/contacts/filter-value-labels", {})).json).toMatchSnapshot()
    const other = await newTenant("p106-ff-other")
    expect((await other.api.post("/v1/contacts/filter-value-labels", { tags: [tagId] })).json).toEqual({ tags: [] })
  })

  it("other id types are accepted (snapshot of the answer shape)", async () => {
    const response = await api.post("/v1/contacts/filter-value-labels", {
      sequences: ["1"],
      inboxes: ["1"],
      flows: ["1"],
      flowNodes: ["1:2"],
      broadcasts: ["1"],
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchSnapshot()
  })

  it("odd ids and long lists (outcome recorded on the original)", async () => {
    const outcome: Record<string, unknown> = {}
    for (const [label, body] of [
      ["non-digit", { tags: ["abc"] }],
      ["leading zero", { tags: ["01"] }],
      ["past bigint", { tags: ["9223372036854775808"] }],
      ["number", { tags: [1] }],
      ["501 ids", { tags: Array.from({ length: 501 }, (_, i) => String(i + 1)) }],
      ["not an array", { tags: "1" }],
    ] as const) {
      const response = await api.post("/v1/contacts/filter-value-labels", body)
      expect(response.status, response.text).toBeLessThan(500)
      outcome[label] = response.status === 200 ? response.json : { status: response.status, code: (response.json as { code?: string }).code }
    }
    expect(outcome).toMatchSnapshot()
  })
})

void tenant
