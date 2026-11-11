// contacts.listCustomFields / getCustomField / setCustomField /
// setCustomFieldLegacy / applyCustomFieldOperations / clearCustomField /
// clearCustomFields: values per field type, operations, errors.

import { beforeAll, describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { expectNormalized, Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  contactPath,
  createCustomField,
  expectError,
  expectInvalid,
  idOf,
  newTenant,
  seedContact,
  sortBy,
  uniqueName,
} from "./helpers.ts"

type FieldValue = { id: string; name: string; type: string; description: string | null; value: string }

let tenant: Tenant
let api: Api
const field: Record<string, { id: string; name: string }> = {}

beforeAll(async () => {
  ;({ tenant, api } = await newTenant("p106-cf"))
  for (const type of ["shortText", "longText", "number", "date", "datetime", "boolean", "email", "phoneNumber"] as const) {
    const name = `f_${type}`
    field[type] = { id: await createCustomField(api, name, type), name }
  }
})

const newContact = async (input: { timezone?: string } = {}) =>
  (await seedContact(tenant.workspaceId, { firstName: "Fields", ...input })).contactId
const path = (contactId: string, suffix = "") => contactPath(idOf(contactId), `/custom-fields${suffix}`)
const valuesOf = async (contactId: string): Promise<Record<string, string>> => {
  const response = await api.get<{ data: FieldValue[] }>(path(contactId))
  expect(response.status, response.text).toBe(200)
  return Object.fromEntries(response.json.data.map((row) => [row.name, row.value]))
}
const set = (contactId: string, key: string, value: string, extra: Record<string, unknown> = {}) =>
  api.put(path(contactId, `/${encodeURIComponent(key)}`), { value, ...extra })

describe("list / get", () => {
  it("lists every value with its field definition; get by id or name", async () => {
    const n = new Normalizer()
    const contactId = await newContact()
    expect(await valuesOf(contactId)).toEqual({})
    expect((await set(contactId, field.shortText.id, "hello")).status).toBe(204)
    expect((await set(contactId, field.number.name, "42")).status).toBe(204)
    const list = await api.get<{ data: FieldValue[] }>(path(contactId))
    const rows = sortBy(list.json.data, (row) => row.name)
    expect(rows).toEqual([
      { id: field.number.id, name: "f_number", type: "number", description: null, value: "42" },
      { id: field.shortText.id, name: "f_shortText", type: "shortText", description: null, value: "hello" },
    ])
    expectNormalized(n, { data: rows }).toMatchSnapshot()

    const byId = await api.get<FieldValue>(path(contactId, `/${field.number.id}`))
    expect(byId.status, byId.text).toBe(200)
    expect(byId.json).toEqual(rows[0])
    const byName = await api.get<FieldValue>(path(contactId, "/f_number"))
    expect(byName.json).toEqual(rows[0])
  })

  it("404s: field without a value on the contact, unknown field, unknown contact", async () => {
    const contactId = await newContact()
    expectError(await api.get(path(contactId, `/${field.date.id}`)), 404, "notFound", {
      message: "Contact custom field not found",
    })
    expectError(await api.get(path(contactId, "/no_such_field")), 404, "notFound", { message: "Custom field not found" })
    expectError(await api.get(contactPath("id:1", "/custom-fields")), 404, "notFound")
    expectError(await api.get(contactPath("id:1", `/custom-fields/${field.date.id}`)), 404, "notFound")
  })
})

describe("setCustomField: stored form per type", () => {
  it.each([
    ["shortText", "  padded  ", "padded"],
    ["longText", "line one\nline two", "line one\nline two"],
    ["number", " 1.50 ", "1.5"],
    ["number", "007", "7"],
    ["number", "-0.25", "-0.25"],
    ["boolean", "yes", "true"],
    ["boolean", "OFF", "false"],
    ["date", "2025-05-01", "2025-05-01T00:00:00Z"],
    ["datetime", "2025-05-01T12:30:00Z", "2025-05-01T12:30:00.000Z"],
    ["email", "A@Parity.Test", "A@Parity.Test"],
    ["phoneNumber", "+84 90 123", "+84 90 123"],
  ])("%s %j -> %j", async (type, input, stored) => {
    const contactId = await newContact()
    const response = await set(contactId, field[type]?.name as string, input)
    expect(response.status, response.text).toBe(204)
    expect((await valuesOf(contactId))[field[type]?.name as string]).toBe(stored)
  })

  it("a naive datetime is anchored to the contact's time zone", async () => {
    const contactId = await newContact({ timezone: "Asia/Ho_Chi_Minh" })
    expect((await set(contactId, field.datetime.name, "2025-05-01T12:30")).status).toBe(204)
    expect((await valuesOf(contactId)).f_datetime).toBe("2025-05-01T05:30:00.000Z")
  })

  it("an empty value is stored as empty", async () => {
    const contactId = await newContact()
    expect((await set(contactId, field.shortText.name, "x")).status).toBe(204)
    expect((await set(contactId, field.shortText.name, "   ")).status).toBe(204)
    expect(await valuesOf(contactId)).toEqual({ f_shortText: "" })
  })

  it("invalid values and input", async () => {
    const contactId = await newContact()
    expectError(await set(contactId, field.number.name, "abc"), 400, "invalidCustomFieldValue", {
      message: '"abc" is not a valid number value for this field.',
    })
    // A non-ISO date is accepted with 204...
    const date = await set(contactId, field.date.name, "05/01/2025")
    expect(date.status, date.text).toBe(204)
    // ...but nothing is stored.
    expect(await valuesOf(contactId)).toEqual({})
    expectInvalid(await set(contactId, field.date.name, "2025-05-01", { clientTimezone: "Mars/Base" }), "clientTimezone")
    expectInvalid(await api.put(path(contactId, `/${field.shortText.name}`), {}), "value")
    expectError(await set(contactId, "no_such_field", "x"), 404, "notFound", { message: "Custom field not found" })
    expectError(await set("1", field.shortText.name, "x"), 404, "notFound")
    expect(await valuesOf(contactId)).toEqual({})
  })
})

describe("setCustomFieldLegacy (POST, numeric id only)", () => {
  it("sets by id; a name or unknown id is refused", async () => {
    const contactId = await newContact()
    const legacy = (key: string, value: string) => api.post(path(contactId, `/${key}`), { value })
    expect((await legacy(field.number.id, " 3 ")).status).toBe(204)
    expect((await valuesOf(contactId)).f_number).toBe("3")
    expectInvalid(await legacy("f_number", "4"), "customFieldId")
    expectError(await legacy("1", "4"), 404, "notFound", { message: "Custom field not found" })
  })
})

describe("applyCustomFieldOperations", () => {
  const apply = (contactId: string, operations: unknown[], extra: Record<string, unknown> = {}) =>
    api.patch(path(contactId), { operations, ...extra })

  it("applies operations in order, by id or name, per type", async () => {
    const contactId = await newContact()
    const response = await apply(contactId, [
      { customFieldId: "f_shortText", operation: "set", value: "mid" },
      { customFieldId: field.shortText.id, operation: "append", value: "-end" },
      { customFieldId: "f_shortText", operation: "prepend", value: "start-" },
      { customFieldId: "f_number", operation: "increase", value: "5" },
      { customFieldId: "f_number", operation: "decrease", value: "1.5" },
      { customFieldId: "f_number", operation: "formula", value: "value * 2" },
      { customFieldId: "f_date", operation: "set", value: "2025-01-31" },
      { customFieldId: "f_date", operation: "addDays", value: "1" },
      { customFieldId: "f_boolean", operation: "set", value: "true" },
      { customFieldId: "f_longText", operation: "set", value: "temp" },
      { customFieldId: "f_longText", operation: "clear" },
    ])
    expect(response.status, response.text).toBe(204)
    expect(await valuesOf(contactId)).toEqual({
      f_shortText: "start-mid-end",
      f_number: "7",
      f_date: "2025-02-01T00:00:00Z",
      f_boolean: "true",
    })
  })

  it("setNow and dateExpression write the current time", async () => {
    const before = Date.now()
    const contactId = await newContact()
    const response = await apply(contactId, [
      { customFieldId: "f_datetime", operation: "setNow" },
      { customFieldId: "f_date", operation: "dateExpression", value: "now + 2 days" },
    ])
    expect(response.status, response.text).toBe(204)
    const values = await valuesOf(contactId)
    const stamp = Date.parse(values.f_datetime as string)
    expect(stamp).toBeGreaterThanOrEqual(before - 5_000)
    expect(stamp).toBeLessThanOrEqual(Date.now() + 5_000)
    expect(values.f_date).toBe(`${new Date(before + 2 * 86_400_000).toISOString().slice(0, 10)}T00:00:00Z`)
  })

  it("an operation the type does not support fails the whole batch (400 invalidFieldOperation)", async () => {
    const contactId = await newContact()
    const response = await apply(contactId, [
      { customFieldId: "f_shortText", operation: "set", value: "kept?" },
      { customFieldId: "f_number", operation: "append", value: "1" },
    ])
    expectError(response, 400, "invalidFieldOperation", {
      message: "This operation cannot be applied to the field (operationNotAllowed). (reason=operationNotAllowed)",
    })
    expect(await valuesOf(contactId)).toEqual({})
    expectError(
      await apply(contactId, [{ customFieldId: "f_number", operation: "formula", value: "value +* 2" }]),
      400,
      "invalidFieldOperation",
    )
  })

  it("an unknown field fails the whole batch (404)", async () => {
    const contactId = await newContact()
    const response = await apply(contactId, [
      { customFieldId: "f_shortText", operation: "set", value: "x" },
      { customFieldId: "no_such_field", operation: "set", value: "y" },
    ])
    expectError(response, 404, "notFound", { message: "Custom field not found" })
    expect(await valuesOf(contactId)).toEqual({})
  })

  it("input validation", async () => {
    const contactId = await newContact()
    expectInvalid(await apply(contactId, []), "operations")
    expectInvalid(
      await apply(
        contactId,
        Array.from({ length: 21 }, () => ({ customFieldId: "f_number", operation: "increase", value: "1" })),
      ),
      "operations",
    )
    expectInvalid(await apply(contactId, [{ customFieldId: "f_number", operation: "multiply", value: "2" }]), "operations.0.operation")
    // A blank operand is a 422 from the input schema, but an omitted one
    // (`value` defaults to "" after the refine ran) reaches the calculator:
    // 400 invalidFieldOperation, reason operandRequired.
    expectInvalid(await apply(contactId, [{ customFieldId: "f_date", operation: "addDays", value: " " }]), "operations.0.value")
    expectError(await apply(contactId, [{ customFieldId: "f_number", operation: "increase" }]), 400, "invalidFieldOperation", {
      message: "This operation cannot be applied to the field (operandRequired). (reason=operandRequired)",
    })
    expectInvalid(await apply(contactId, [{ customFieldId: "", operation: "clear" }]), "operations.0.customFieldId")
    expectInvalid(
      await apply(contactId, [{ customFieldId: "f_date", operation: "set", value: "2025-01-01" }], { clientTimezone: "x/y" }),
      "clientTimezone",
    )
    expectError(await api.patch(contactPath("id:1", "/custom-fields"), { operations: [{ customFieldId: "f_number", operation: "clear" }] }), 404, "notFound")
  })
})

describe("clearCustomField / clearCustomFields", () => {
  it("clears one value by id or name, then all values", async () => {
    const contactId = await newContact()
    for (const [key, value] of [
      ["f_shortText", "a"],
      ["f_number", "1"],
      ["f_boolean", "true"],
    ] as const) {
      expect((await set(contactId, key, value)).status).toBe(204)
    }
    expect((await api.delete(path(contactId, "/f_shortText"))).status).toBe(204)
    expect((await api.delete(path(contactId, `/${field.number.id}`))).status).toBe(204)
    expect(await valuesOf(contactId)).toEqual({ f_boolean: "true" })
    // A field the contact has no value for: still 204.
    expect((await api.delete(path(contactId, "/f_date"))).status).toBe(204)
    expectError(await api.delete(path(contactId, "/no_such_field")), 404, "notFound", { message: "Custom field not found" })
    expect((await api.delete(path(contactId))).status).toBe(204)
    expect(await valuesOf(contactId)).toEqual({})
    expect((await api.delete(path(contactId))).status).toBe(204)
    expectError(await api.delete(contactPath("id:1", "/custom-fields")), 404, "notFound")
    expectError(await api.delete(contactPath("id:1", "/custom-fields/f_date")), 404, "notFound")
  })

  it("a field of another workspace is unknown here", async () => {
    const contactId = await newContact()
    const other = await newTenant("p106-cf-other")
    const foreign = await createCustomField(other.api, uniqueName("foreign"), "shortText")
    expectError(await set(contactId, foreign, "x"), 404, "notFound")
    expectError(await api.get(path(contactId, `/${foreign}`)), 404, "notFound")
    expectError(await api.delete(path(contactId, `/${foreign}`)), 404, "notFound")
  })
})
