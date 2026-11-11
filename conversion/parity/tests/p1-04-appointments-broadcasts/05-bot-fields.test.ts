// botFields.* (list, create, get, set, update, setMany, bulkUpdate,
// applyOperation, reset, resetMany, delete).

import { describe, expect, it } from "vitest"
import { Normalizer } from "../../src/normalize.ts"
import { type Api, errorShape, expectError, expectInvalid, newTenant } from "./helpers.ts"

type BotField = {
  id: string
  name: string
  type: string
  value: string | null
  description: string | null
  folderId: string | null
  createdAt: string
  updatedAt: string
}

const BOT_FIELD_KEYS = ["createdAt", "description", "folderId", "id", "name", "type", "updatedAt", "value"]

const createField = async (api: Api, body: Record<string, unknown>): Promise<BotField> => {
  const response = await api.post<BotField>("/v1/bot-fields", body)
  expect(response.status, response.text).toBe(201)
  return response.json
}

const getField = async (api: Api, key: string): Promise<BotField> => {
  const response = await api.get<BotField>(`/v1/bot-fields/${encodeURIComponent(key)}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("botFields.create", () => {
  it("creates a field (201) with null value and description by default", async () => {
    const normalizer = new Normalizer()
    const { api } = await newTenant("bf-create")
    const field = await createField(api, { name: "  shop_name  ", type: "shortText" })
    expect(field).toMatchObject({
      name: "shop_name",
      type: "shortText",
      value: null,
      description: null,
      folderId: null,
    })
    expect(Object.keys(field).sort()).toEqual(BOT_FIELD_KEYS)
    expect(normalizer.normalize(field)).toMatchSnapshot("created")

    const withValue = await createField(api, {
      name: "opening_count",
      type: "number",
      value: " 007 ",
      description: "How often",
    })
    expect(withValue).toMatchObject({ type: "number", description: "How often" })
    expect({ value: withValue.value }).toMatchSnapshot("normalized number value")
  })

  it("rejects bad input with 422 and a duplicate name of the same type", async () => {
    const { api } = await newTenant("bf-create-invalid")
    const missing = await api.post("/v1/bot-fields", {})
    expectInvalid(missing, "name")
    expectInvalid(missing, "type")
    expectInvalid(await api.post("/v1/bot-fields", { name: "x", type: "color" }), "type")
    expectInvalid(await api.post("/v1/bot-fields", { name: "", type: "shortText" }), "name")
    expectInvalid(await api.post("/v1/bot-fields", { name: "bot_field:12", type: "shortText" }), "name")
    expectInvalid(await api.post("/v1/bot-fields", { name: "x".repeat(256), type: "shortText" }), "name")
    expectInvalid(await api.post("/v1/bot-fields", { name: "x", type: "shortText", value: "v".repeat(1001) }), "value")
    expectInvalid(await api.post("/v1/bot-fields", { name: "x", type: "shortText", folderId: "abc" }), "folderId")

    await createField(api, { name: "taken", type: "shortText" })
    const duplicate = await api.post("/v1/bot-fields", { name: "taken", type: "shortText" })
    const body = expectError(duplicate, 422, "validation")
    expect(body.message).toBe("Name is already taken")
    expect(errorShape(duplicate)).toMatchSnapshot("duplicate name")
    // The same name with another type is a different field.
    await createField(api, { name: "taken", type: "number" })

    const badNumber = await api.post("/v1/bot-fields", { name: "n", type: "number", value: "abc" })
    const numberBody = expectError(badNumber, 400, "invalidCustomFieldValue")
    expect(numberBody.message).toBe('"abc" is not a valid number value for this field.')

    const unknownFolder = await api.post("/v1/bot-fields", { name: "f", type: "shortText", folderId: "1" })
    expect(unknownFolder.status).toBeGreaterThanOrEqual(400)
    expect(errorShape(unknownFolder)).toMatchSnapshot("unknown folder")
  })
})

describe("botFields.list", () => {
  it("lists fields newest first, filters by name and folder, sorts and pages", async () => {
    const normalizer = new Normalizer()
    const { api } = await newTenant("bf-list")
    const a = await createField(api, { name: "alpha_field", type: "shortText", value: "a" })
    const b = await createField(api, { name: "beta_field", type: "number", value: "2" })
    const c = await createField(api, { name: "gamma_field", type: "boolean" })

    const all = await api.get<{ data: BotField[]; pageCount: number }>("/v1/bot-fields")
    expect(all.status, all.text).toBe(200)
    expect(all.json.pageCount).toBe(1)
    expect(all.json.data.map((field) => field.id)).toEqual([c.id, b.id, a.id])
    expect(normalizer.normalize(all.json)).toMatchSnapshot("list")

    const byName = await api.get<{ data: BotField[] }>("/v1/bot-fields", { query: { name: "BETA" } })
    expect(byName.json.data.map((field) => field.id)).toEqual([b.id])
    const root = await api.get<{ data: BotField[] }>("/v1/bot-fields", { query: { folderId: "0" } })
    expect(root.json.data).toHaveLength(3)

    const sorted = await api.get<{ data: BotField[] }>(
      "/v1/bot-fields?sort%5B0%5D%5Bid%5D=name&sort%5B0%5D%5Bdesc%5D=false",
    )
    expect(sorted.status, sorted.text).toBe(200)
    expect(sorted.json.data.map((field) => field.name)).toEqual(["alpha_field", "beta_field", "gamma_field"])

    const page = await api.get<{ data: BotField[]; pageCount: number }>("/v1/bot-fields", {
      query: { perPage: 2, page: 2 },
    })
    expect(page.json.pageCount).toBe(2)
    expect(page.json.data.map((field) => field.id)).toEqual([a.id])

    expectInvalid(await api.get("/v1/bot-fields", { query: { folderId: "x" } }), "folderId")
    expectInvalid(await api.get("/v1/bot-fields", { query: { perPage: 51 } }), "perPage")
  })
})

describe("botFields.get", () => {
  it("finds a field by id or by name; 404 otherwise", async () => {
    const { api } = await newTenant("bf-get")
    const field = await createField(api, { name: "lookup_me", type: "shortText", value: "v" })
    expect(await getField(api, field.id)).toEqual(field)
    expect(await getField(api, "lookup_me")).toEqual(field)
    const unknown = await api.get("/v1/bot-fields/nobody")
    const body = expectError(unknown, 404, "notFound")
    expect(body.message).toBe("Bot field not found")
    expect(errorShape(unknown)).toMatchSnapshot("404 shape")
    expectError(await api.get("/v1/bot-fields/1"), 404, "notFound")
  })
})

describe("botFields.set", () => {
  it("sets the value by id or name", async () => {
    const { api } = await newTenant("bf-set")
    const field = await createField(api, { name: "greeting", type: "shortText" })
    const byId = await api.put<BotField>(`/v1/bot-fields/${field.id}`, { value: "Hello" })
    expect(byId.status, byId.text).toBe(200)
    expect(byId.json).toMatchObject({ id: field.id, value: "Hello" })
    const byName = await api.put<BotField>("/v1/bot-fields/greeting", { value: "Hi" })
    expect(byName.json.value).toBe("Hi")
    expect((await getField(api, field.id)).value).toBe("Hi")
  })

  it("validates the value against the type and the body", async () => {
    const { api } = await newTenant("bf-set-invalid")
    await createField(api, { name: "count", type: "number" })
    const many = expectError(await api.put("/v1/bot-fields/count", { value: "many" }), 400, "invalidCustomFieldValue")
    expect(many.message).toBe('"many" is not a valid number value for this field.')
    expectInvalid(await api.put("/v1/bot-fields/count", {}), "value")
    expectInvalid(await api.put("/v1/bot-fields/count", { value: "1".repeat(1001) }), "value")
    expectError(await api.put("/v1/bot-fields/missing", { value: "1" }), 404, "notFound")
    const set = await api.put<BotField>("/v1/bot-fields/count", { value: "42" })
    expect(set.json.value).toBe("42")
  })
})

describe("botFields.update", () => {
  it("changes name, description and value; an empty patch returns the field", async () => {
    const { api } = await newTenant("bf-update")
    const field = await createField(api, { name: "old_name", type: "shortText", value: "v1" })
    const response = await api.patch<BotField>(`/v1/bot-fields/${field.id}`, {
      name: "new_name",
      description: "Described",
      value: "v2",
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ id: field.id, name: "new_name", description: "Described", value: "v2" })
    expectError(await api.get("/v1/bot-fields/old_name"), 404, "notFound")

    const unchanged = await api.patch<BotField>("/v1/bot-fields/new_name", {})
    expect(unchanged.status).toBe(200)
    expect(unchanged.json).toEqual(response.json)
  })

  it("rejects a taken name, a value that does not fit and unknown fields", async () => {
    const { api } = await newTenant("bf-update-invalid")
    await createField(api, { name: "first", type: "shortText" })
    const second = await createField(api, { name: "second", type: "shortText" })
    const taken = await api.patch(`/v1/bot-fields/${second.id}`, { name: "first" })
    const body = expectError(taken, 422, "validation")
    expect(body.message).toBe("Name is already taken")
    expectError(await api.patch(`/v1/bot-fields/${second.id}`, { type: "number", value: "x" }), 400, "invalidCustomFieldValue")
    expectInvalid(await api.patch(`/v1/bot-fields/${second.id}`, { type: "color" }), "type")
    expectError(await api.patch("/v1/bot-fields/unknown", { value: "1" }), 404, "notFound")
  })
})

describe("botFields.setMany", () => {
  it("sets several values by id, name or key (204)", async () => {
    const { api } = await newTenant("bf-setmany")
    const a = await createField(api, { name: "field_a", type: "shortText" })
    await createField(api, { name: "field_b", type: "number" })
    await createField(api, { name: "field_c", type: "shortText" })
    const response = await api.put("/v1/bot-fields", {
      fields: [
        { id: a.id, value: "A" },
        { name: "field_b", value: 12 },
        { key: "field_c", value: "C" },
      ],
    })
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expect((await getField(api, "field_a")).value).toBe("A")
    expect((await getField(api, "field_b")).value).toBe("12")
    expect((await getField(api, "field_c")).value).toBe("C")
  })

  it("fails for an unknown entry and rejects a bad body", async () => {
    const { api } = await newTenant("bf-setmany-invalid")
    await createField(api, { name: "known", type: "shortText" })
    const response = await api.put("/v1/bot-fields", {
      fields: [
        { name: "known", value: "written" },
        { name: "unknown", value: "x" },
      ],
    })
    expectError(response, 404, "notFound")
    expectInvalid(await api.put("/v1/bot-fields", {}), "fields")
    expectInvalid(await api.put("/v1/bot-fields", { fields: [{ value: "x" }] }), "fields.0")
  })
})

describe("botFields.bulkUpdate (deprecated)", () => {
  it("sets several values like setMany (204)", async () => {
    const { api } = await newTenant("bf-bulkupdate")
    const a = await createField(api, { name: "bulk_a", type: "shortText" })
    await createField(api, { name: "bulk_b", type: "shortText" })
    const response = await api.put("/v1/bot-fields/bulk-update", {
      fields: [
        { id: a.id, value: "one" },
        { key: "bulk_b", value: 2 },
      ],
    })
    expect(response.status, response.text).toBe(204)
    expect((await getField(api, "bulk_a")).value).toBe("one")
    expect((await getField(api, "bulk_b")).value).toBe("2")
    expectInvalid(await api.put("/v1/bot-fields/bulk-update", { fields: "x" }), "fields")
    expectError(
      await api.put("/v1/bot-fields/bulk-update", { fields: [{ name: "nope", value: "1" }] }),
      404,
      "notFound",
    )
  })
})

describe("botFields.applyOperation", () => {
  it("applies number and text operations", async () => {
    const { api } = await newTenant("bf-operation")
    const counter = await createField(api, { name: "counter", type: "number", value: "10" })
    const apply = (key: string, body: Record<string, unknown>) =>
      api.post<BotField>(`/v1/bot-fields/${key}/operations`, body)

    const increased = await apply(counter.id, { operation: "increase", value: "5" })
    expect(increased.status, increased.text).toBe(200)
    expect(increased.json).toMatchObject({ id: counter.id, value: "15" })
    expect((await apply("counter", { operation: "decrease", value: "2.5" })).json.value).toBe("12.5")
    expect((await apply("counter", { operation: "set", value: "3" })).json.value).toBe("3")
    const formula = await apply("counter", { operation: "formula", value: "value * 2 + 1" })
    expect(formula.status, formula.text).toBe(200)
    expect(formula.json.value).toBe("7")
    expect((await apply("counter", { operation: "clear" })).json.value).toBeNull()

    await createField(api, { name: "label", type: "shortText", value: "mid" })
    expect((await apply("label", { operation: "append", value: "-end" })).json.value).toBe("mid-end")
    expect((await apply("label", { operation: "prepend", value: "start-" })).json.value).toBe("start-mid-end")
    expect((await getField(api, "label")).value).toBe("start-mid-end")
  })

  it("rejects an operation the type does not support, a missing operand and unknown operations", async () => {
    const { api } = await newTenant("bf-operation-invalid")
    await createField(api, { name: "counter", type: "number", value: "1" })
    await createField(api, { name: "label", type: "shortText", value: "x" })
    const wrongType = await api.post("/v1/bot-fields/counter/operations", { operation: "append", value: "1" })
    const body = expectError(wrongType, 400, "invalidFieldOperation")
    expect(errorShape(wrongType)).toMatchSnapshot("unsupported operation")
    expect(body.message).toMatch(/not supported for field type "number"/)
    expectError(
      await api.post("/v1/bot-fields/label/operations", { operation: "increase", value: "1" }),
      400,
      "invalidFieldOperation",
    )
    expectError(
      await api.post("/v1/bot-fields/counter/operations", { operation: "increase", value: "abc" }),
      400,
      "invalidFieldOperation",
    )
    // A missing operand is checked by the service, not the schema, for increase.
    const noOperand = await api.post("/v1/bot-fields/counter/operations", { operation: "increase" })
    expect(expectError(noOperand, 400, "invalidFieldOperation").message).toBe('"" is not a valid number value for this field.')
    // append without a value is a no-op.
    const noAppend = await api.post<{ value: string | null }>("/v1/bot-fields/label/operations", { operation: "append" })
    expect(noAppend.status, noAppend.text).toBe(200)
    expect(noAppend.json.value).toBe("x")
    expectInvalid(await api.post("/v1/bot-fields/counter/operations", { operation: "explode", value: "1" }), "operation")
    expectInvalid(
      await api.post("/v1/bot-fields/counter/operations", { operation: "set", value: "1", clientTimezone: "Mars/Base" }),
      "clientTimezone",
    )
    expectError(await api.post("/v1/bot-fields/missing/operations", { operation: "clear" }), 404, "notFound")
  })
})

describe("botFields.reset", () => {
  it("clears one value and keeps the field", async () => {
    const { api } = await newTenant("bf-reset")
    const field = await createField(api, { name: "resettable", type: "shortText", value: "keep?" })
    const response = await api.post<BotField>(`/v1/bot-fields/${field.id}/reset`, {})
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ id: field.id, name: "resettable", value: null })
    expect((await getField(api, "resettable")).value).toBeNull()
    expectError(await api.post("/v1/bot-fields/missing/reset", {}), 404, "notFound")
  })
})

describe("botFields.resetMany", () => {
  it("clears several values (204) and ignores foreign ids", async () => {
    const { api } = await newTenant("bf-resetmany")
    const a = await createField(api, { name: "ra", type: "shortText", value: "1" })
    const b = await createField(api, { name: "rb", type: "shortText", value: "2" })
    const c = await createField(api, { name: "rc", type: "shortText", value: "3" })
    const { api: other } = await newTenant("bf-resetmany-other")
    const foreign = await createField(other, { name: "foreign", type: "shortText", value: "stay" })

    const response = await api.post("/v1/bot-fields/bulk-reset", { ids: [a.id, b.id, foreign.id] })
    expect(response.status, response.text).toBe(204)
    expect((await getField(api, a.id)).value).toBeNull()
    expect((await getField(api, b.id)).value).toBeNull()
    expect((await getField(api, c.id)).value).toBe("3")
    expect((await getField(other, foreign.id)).value).toBe("stay")
  })

  it("rejects an empty, oversized or non-numeric id list with 422", async () => {
    const { api } = await newTenant("bf-resetmany-invalid")
    expectInvalid(await api.post("/v1/bot-fields/bulk-reset", { ids: [] }), "ids")
    expectInvalid(await api.post("/v1/bot-fields/bulk-reset", {}), "ids")
    expectInvalid(
      await api.post("/v1/bot-fields/bulk-reset", { ids: Array.from({ length: 101 }, (_, index) => String(index + 1)) }),
      "ids",
    )
    expectInvalid(await api.post("/v1/bot-fields/bulk-reset", { ids: ["abc"] }), "ids.0")
  })
})

describe("botFields.delete", () => {
  it("deletes the field (204); get and delete are then 404", async () => {
    const { api } = await newTenant("bf-delete")
    const field = await createField(api, { name: "doomed", type: "shortText" })
    const response = await api.delete(`/v1/bot-fields/${field.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectError(await api.get(`/v1/bot-fields/${field.id}`), 404, "notFound")
    expectError(await api.delete("/v1/bot-fields/doomed"), 404, "notFound")
    expect((await api.get<{ data: unknown[] }>("/v1/bot-fields")).json.data).toEqual([])

    // By name.
    await createField(api, { name: "by_name", type: "shortText" })
    expect((await api.delete("/v1/bot-fields/by_name")).status).toBe(204)
  })
})
