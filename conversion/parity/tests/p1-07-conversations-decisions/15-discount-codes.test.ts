// discountCodes.list / get / generateCode / create / update / setActive /
// delete / validate (scope `ecommerce`). `validate` prices an order like
// `orders.create`; its products come from the public `products.create`.

import { describe, expect, it } from "vitest"
import { Normalizer } from "../../src/normalize.ts"
import { expectError, expectInvalid, idOf, newTenant, qs, UNKNOWN_ID, type Api } from "./helpers.ts"
import { createProduct, newContact, unique } from "./helpers-ecommerce.ts"

type DiscountCode = {
  id: string
  code: string
  autoApply: boolean
  discountType: string
  discountValue: number
  appliesTo: string
  categoryId: string | null
  productIds: string[]
  minimumType: string
  minimumValue: number
  usageLimit: number | null
  usedCount: number
  oncePerCustomer: boolean
  eligibility: string
  customerFilter: unknown
  startsAt: string
  endsAt: string | null
  isActive: boolean
  createdAt: string
}
type Validation = {
  valid: boolean
  amount: number
  currency: string
  error: { code: string; data?: Record<string, string | number> } | null
  total: number
}

const KEYS = [
  "appliesTo",
  "autoApply",
  "categoryId",
  "code",
  "createdAt",
  "customerFilter",
  "discountType",
  "discountValue",
  "eligibility",
  "endsAt",
  "id",
  "isActive",
  "minimumType",
  "minimumValue",
  "oncePerCustomer",
  "productIds",
  "startsAt",
  "usageLimit",
  "usedCount",
]

const DAY_MS = 24 * 60 * 60 * 1000

const create = async (api: Api, body: Record<string, unknown>): Promise<DiscountCode> => {
  const response = await api.post<DiscountCode>("/v1/discount-codes", { code: unique("C"), discountType: "percentage", discountValue: 10, ...body })
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("discountCodes.generateCode", () => {
  it("suggests an 8-character code from the unambiguous alphabet; creates nothing", async () => {
    const { api } = await newTenant("p107-dc-generate")
    const response = await api.get<{ code: string }>("/v1/discount-codes/generate-code")
    expect(response.status, response.text).toBe(200)
    expect(Object.keys(response.json)).toEqual(["code"])
    expect(response.json.code).toMatch(/^[A-HJ-NP-Za-km-z2-9]{8}$/)
    expect((await api.get<{ data: unknown[] }>("/v1/discount-codes")).json.data).toEqual([])
  })
})

describe("discountCodes.create / get", () => {
  it("creates a code with the defaults (200) and reads it back", async () => {
    const n = new Normalizer()
    const { api } = await newTenant("p107-dc-create")
    const code = unique("Save")
    const created = await api.post<DiscountCode>("/v1/discount-codes", {
      code: `  ${code}  `,
      discountType: "percentage",
      discountValue: 15,
    })
    expect(created.status, created.text).toBe(200)
    expect(Object.keys(created.json).sort()).toEqual(KEYS)
    expect(n.normalize(created.json)).toEqual({
      id: "<id:1>",
      code,
      autoApply: false,
      discountType: "percentage",
      discountValue: 15,
      appliesTo: "entireOrder",
      categoryId: null,
      productIds: [],
      minimumType: "none",
      minimumValue: 0,
      usageLimit: null,
      usedCount: 0,
      oncePerCustomer: false,
      eligibility: "everyone",
      customerFilter: null,
      startsAt: "<ts>",
      endsAt: null,
      isActive: true,
      createdAt: "<ts>",
    })
    const fetched = await api.get<DiscountCode>(`/v1/discount-codes/${created.json.id}`)
    expect(fetched.status, fetched.text).toBe(200)
    expect(fetched.json).toEqual(created.json)
  })

  it("stores every field; normalizes free shipping, unused minimum and product duplicates", async () => {
    const { api } = await newTenant("p107-dc-create-full")
    const product = await createProduct(api, { price: 50 })
    const startsAt = new Date(Date.now() - DAY_MS).toISOString()
    const endsAt = new Date(Date.now() + 30 * DAY_MS).toISOString()
    const full = await create(api, {
      autoApply: true,
      discountType: "fixedAmount",
      discountValue: 7.5,
      appliesTo: "products",
      productIds: [product, product],
      minimumType: "quantity",
      minimumValue: 2,
      usageLimit: 5,
      oncePerCustomer: true,
      startsAt,
      endsAt,
      isActive: false,
    })
    expect(full).toMatchObject({
      autoApply: true,
      discountType: "fixedAmount",
      discountValue: 7.5,
      appliesTo: "products",
      productIds: [product],
      minimumType: "quantity",
      minimumValue: 2,
      usageLimit: 5,
      oncePerCustomer: true,
      startsAt,
      endsAt,
      isActive: false,
    })
    const shipping = await create(api, {
      discountType: "freeShipping",
      discountValue: 30,
      appliesTo: "products",
      productIds: [product],
      minimumType: "none",
      minimumValue: 9,
    })
    expect(shipping).toMatchObject({
      discountType: "freeShipping",
      discountValue: 0,
      appliesTo: "entireOrder",
      productIds: [],
      minimumValue: 0,
    })
    const filtered = await create(api, {
      eligibility: "specificCustomers",
      customerFilter: { operator: "and", conditions: [{ field: "gender", operator: "eq", value: "female" }] },
    })
    expect(filtered).toMatchObject({
      eligibility: "specificCustomers",
      customerFilter: { operator: "and", conditions: [{ field: "gender", operator: "eq", value: "female" }] },
    })
    // Without specificCustomers the filter is dropped.
    const everyone = await create(api, {
      customerFilter: { operator: "and", conditions: [{ field: "gender", operator: "eq", value: "female" }] },
    })
    expect(everyone.customerFilter).toBeNull()
  })

  it("409 discountCodeDuplicated for the same code in any case; other workspaces are separate", async () => {
    const { api } = await newTenant("p107-dc-duplicate")
    const first = await create(api, {})
    expectError(
      await api.post("/v1/discount-codes", { code: first.code.toLowerCase(), discountType: "percentage", discountValue: 5 }),
      409,
      "discountCodeDuplicated",
      { message: "A discount code with this name already exists." },
    )
    const other = await newTenant("p107-dc-duplicate-other")
    expect((await other.api.post("/v1/discount-codes", { code: first.code, discountType: "percentage", discountValue: 5 })).status).toBe(200)
  })

  it("422 invalidRequestData for the schema, 422 validation for the business rules", async () => {
    const { api } = await newTenant("p107-dc-create-errors")
    const post = (body: Record<string, unknown>) =>
      api.post("/v1/discount-codes", { code: unique("V"), discountType: "percentage", discountValue: 10, ...body })
    expectInvalid(await post({ code: "ab" }), "code")
    expectInvalid(await post({ code: "has space" }), "code")
    expectInvalid(await post({ code: "x".repeat(65) }), "code")
    expectInvalid(await post({ discountType: "bogo" }), "discountType")
    expectInvalid(await post({ discountValue: -1 }), "discountValue")
    expectInvalid(await post({ usageLimit: 0 }), "usageLimit")
    expectInvalid(await post({ productIds: ["abc"] }), "productIds.0")
    expectInvalid(await post({ startsAt: "nope" }), "startsAt")
    expectInvalid(await api.post("/v1/discount-codes", { code: unique("V"), discountValue: 1 }), "discountType")
    const rule = (body: Record<string, unknown>, message: string) =>
      post(body).then((response) => expectError(response, 422, "validation", { message }))
    await rule({ discountValue: 0 }, "A percentage is more than 0 and at most 100.")
    await rule({ discountValue: 101 }, "A percentage is more than 0 and at most 100.")
    await rule({ discountType: "fixedAmount", discountValue: 0 }, "An amount is more than 0.")
    await rule({ appliesTo: "category" }, "Choose a category of this workspace.")
    await rule({ appliesTo: "category", categoryId: UNKNOWN_ID }, "Choose a category of this workspace.")
    await rule({ appliesTo: "products" }, "Choose products of this workspace.")
    await rule({ appliesTo: "products", productIds: [UNKNOWN_ID] }, "Choose products of this workspace.")
    await rule({ minimumType: "amount" }, "A minimum is more than 0.")
    await rule({ minimumType: "quantity", minimumValue: 1.5 }, "A minimum number of items is a whole number.")
    await rule({ eligibility: "specificCustomers" }, "Add at least one condition.")
    const now = Date.now()
    await rule(
      { startsAt: new Date(now).toISOString(), endsAt: new Date(now - DAY_MS).toISOString() },
      "The end date is after the start date.",
    )
    expect((await api.get<{ data: unknown[] }>("/v1/discount-codes")).json.data).toEqual([])
  })

  it("get 404 for unknown and foreign ids; 422 for a non-numeric id", async () => {
    const { api } = await newTenant("p107-dc-get")
    const other = await newTenant("p107-dc-get-other")
    const foreign = await create(other.api, {})
    expectError(await api.get(`/v1/discount-codes/${UNKNOWN_ID}`), 404, "notFound", { message: "Discount code does not exist." })
    expectError(await api.get(`/v1/discount-codes/${foreign.id}`), 404, "notFound")
    expectInvalid(await api.get("/v1/discount-codes/abc"), "id")
  })
})

describe("discountCodes.list", () => {
  it("lists newest first; code filter ignores case; page/perPage; no paging lists every code", async () => {
    const { api } = await newTenant("p107-dc-list")
    const first = await create(api, { code: unique("Alpha") })
    const second = await create(api, { code: unique("Beta") })
    const third = await create(api, { code: unique("AlphaTwo") })
    const all = await api.get<{ data: DiscountCode[]; pageCount: number }>("/v1/discount-codes")
    expect(all.status, all.text).toBe(200)
    expect(all.json.pageCount).toBe(1)
    expect(all.json.data.map((c) => c.id)).toEqual([third.id, second.id, first.id])
    expect(all.json.data[0]).toEqual(third)
    const alpha = await api.get<{ data: DiscountCode[] }>(`/v1/discount-codes${qs({ code: "ALPHA" })}`)
    expect(alpha.json.data.map((c) => c.id)).toEqual([third.id, first.id])
    const page = await api.get<{ data: DiscountCode[]; pageCount: number }>(`/v1/discount-codes${qs({ page: 2, perPage: 2 })}`)
    expect(page.status, page.text).toBe(200)
    expect(page.json.pageCount).toBe(2)
    expect(page.json.data.map((c) => c.id)).toEqual([first.id])
    expectInvalid(await api.get(`/v1/discount-codes${qs({ perPage: 101 })}`), "perPage")
    expectInvalid(await api.get(`/v1/discount-codes${qs({ code: "x".repeat(65) })}`), "code")
    const other = await newTenant("p107-dc-list-other")
    expect((await other.api.get("/v1/discount-codes")).json).toEqual({ data: [], pageCount: 1 })
  })
})

describe("discountCodes.update", () => {
  it("changes only the fields sent; null clears; the result is checked as a whole", async () => {
    const { api } = await newTenant("p107-dc-update")
    const code = await create(api, { usageLimit: 3, isActive: false, endsAt: new Date(Date.now() + DAY_MS).toISOString() })
    const updated = await api.patch<DiscountCode>(`/v1/discount-codes/${code.id}`, { discountValue: 25 })
    expect(updated.status, updated.text).toBe(200)
    expect(Object.keys(updated.json).sort()).toEqual(KEYS)
    expect(updated.json).toEqual({ ...code, discountValue: 25 })
    const cleared = await api.patch<DiscountCode>(`/v1/discount-codes/${code.id}`, { usageLimit: null, endsAt: null })
    expect(cleared.json).toEqual({ ...code, discountValue: 25, usageLimit: null, endsAt: null })
    const renamed = unique("Renamed")
    expect((await api.patch<DiscountCode>(`/v1/discount-codes/${code.id}`, { code: renamed })).json.code).toBe(renamed)
    expect((await api.get<DiscountCode>(`/v1/discount-codes/${code.id}`)).json).toMatchObject({ code: renamed, discountValue: 25 })
    // Switching type re-checks the stored value.
    expectError(await api.patch(`/v1/discount-codes/${code.id}`, { discountValue: 150 }), 422, "validation", {
      message: "A percentage is more than 0 and at most 100.",
    })
    const asShipping = await api.patch<DiscountCode>(`/v1/discount-codes/${code.id}`, { discountType: "freeShipping" })
    expect(asShipping.json).toMatchObject({ discountType: "freeShipping", discountValue: 0 })
  })

  it("409 duplicate code, 404 unknown / foreign id, 422 input", async () => {
    const { api } = await newTenant("p107-dc-update-errors")
    const other = await newTenant("p107-dc-update-other")
    const foreign = await create(other.api, {})
    const a = await create(api, {})
    const b = await create(api, {})
    expectError(await api.patch(`/v1/discount-codes/${b.id}`, { code: a.code.toUpperCase() }), 409, "discountCodeDuplicated")
    expectError(await api.patch(`/v1/discount-codes/${UNKNOWN_ID}`, { discountValue: 5 }), 404, "notFound", {
      message: "Discount code does not exist.",
    })
    expectError(await api.patch(`/v1/discount-codes/${foreign.id}`, { discountValue: 5 }), 404, "notFound")
    expectInvalid(await api.patch(`/v1/discount-codes/${a.id}`, { code: "a b" }), "code")
    expectInvalid(await api.patch(`/v1/discount-codes/${a.id}`, { isActive: "yes" }), "isActive")
    // An empty patch is a no-op.
    const same = await api.patch<DiscountCode>(`/v1/discount-codes/${a.id}`, {})
    expect(same.status, same.text).toBe(200)
    expect(same.json).toEqual(a)
  })
})

describe("discountCodes.setActive / delete", () => {
  it("turns a code off and on; delete returns the row and removes it", async () => {
    const { api } = await newTenant("p107-dc-active")
    const code = await create(api, {})
    const off = await api.patch<DiscountCode>(`/v1/discount-codes/${code.id}/active`, { isActive: false })
    expect(off.status, off.text).toBe(200)
    expect(off.json).toEqual({ ...code, isActive: false })
    const on = await api.patch<DiscountCode>(`/v1/discount-codes/${code.id}/active`, { isActive: true })
    expect(on.json).toEqual(code)
    expectInvalid(await api.patch(`/v1/discount-codes/${code.id}/active`, {}), "isActive")
    expectError(await api.patch(`/v1/discount-codes/${UNKNOWN_ID}/active`, { isActive: false }), 404, "notFound", {
      message: "Discount code does not exist.",
    })
    expectError(await api.patch(`/v1/discount-codes/${UNKNOWN_ID}/active`, { isActive: true }), 404, "notFound")

    const deleted = await api.delete<DiscountCode>(`/v1/discount-codes/${code.id}`)
    expect(deleted.status, deleted.text).toBe(200)
    expect(deleted.json).toEqual(code)
    expectError(await api.get(`/v1/discount-codes/${code.id}`), 404, "notFound")
    expectError(await api.delete(`/v1/discount-codes/${code.id}`), 404, "notFound", { message: "Discount code does not exist." })
    expect((await api.get<{ data: unknown[] }>("/v1/discount-codes")).json.data).toEqual([])
    // The code is free again.
    expect((await api.post("/v1/discount-codes", { code: code.code, discountType: "percentage", discountValue: 5 })).status).toBe(200)
  })

  it("turning on a code whose rules no longer hold is refused (422 validation)", async () => {
    const { api } = await newTenant("p107-dc-active-invalid")
    const product = await createProduct(api, { price: 10 })
    const code = await create(api, { appliesTo: "products", productIds: [product], isActive: false })
    expect((await api.delete(`/v1/products/${product}`)).status).toBeLessThan(300)
    expectError(await api.patch(`/v1/discount-codes/${code.id}/active`, { isActive: true }), 422, "validation", {
      message: "Choose products of this workspace.",
    })
  })
})

describe("discountCodes.validate", () => {
  it("prices the order like checkout: valid code -> amount and total; unknown code -> error, full total", async () => {
    const { api } = await newTenant("p107-dc-validate")
    const product = await createProduct(api, { price: 40 })
    const code = await create(api, { code: unique("Ten"), discountValue: 10 })
    const items = [{ productId: product, quantity: 2 }]
    const ok = await api.post<Validation>("/v1/discount-codes/validate", { code: code.code.toLowerCase(), items })
    expect(ok.status, ok.text).toBe(200)
    expect(Object.keys(ok.json).sort()).toEqual(["amount", "currency", "error", "total", "valid"])
    expect(ok.json).toMatchObject({ valid: true, amount: 8, error: null })
    const unknown = await api.post<Validation>("/v1/discount-codes/validate", { code: "NOPE-NOPE", items })
    expect(unknown.status, unknown.text).toBe(200)
    expect(unknown.json).toEqual({
      valid: false,
      amount: 0,
      currency: ok.json.currency,
      error: { code: "discountCodeNotFound" },
      total: ok.json.total + 8,
    })
    const fixed = await create(api, { discountType: "fixedAmount", discountValue: 500 })
    // A fixed amount never takes more than the lines are worth.
    const capped = await api.post<Validation>("/v1/discount-codes/validate", { code: fixed.code, items })
    expect(capped.json).toMatchObject({ valid: true, amount: 80, error: null, total: unknown.json.total - 80 })
    // Nothing was used up.
    expect((await api.get<DiscountCode>(`/v1/discount-codes/${code.id}`)).json.usedCount).toBe(0)
  })

  it("reports why a code cannot be used (error.code, error.data)", async () => {
    const { api } = await newTenant("p107-dc-validate-errors")
    const product = await createProduct(api, { price: 20 })
    const otherProduct = await createProduct(api, { price: 5 })
    const items = [{ productId: product, quantity: 1 }]
    const check = async (body: Record<string, unknown>) => {
      const code = await create(api, body)
      const response = await api.post<Validation>("/v1/discount-codes/validate", { code: code.code, items })
      expect(response.status, response.text).toBe(200)
      expect(response.json).toMatchObject({ valid: false, amount: 0 })
      return response.json.error
    }
    expect(await check({ isActive: false })).toEqual({ code: "discountCodeInactive" })
    const startsAt = new Date(Date.now() + 2 * DAY_MS).toISOString()
    expect(await check({ startsAt })).toEqual({ code: "discountCodeNotStarted", data: { startsAt } })
    expect(
      await check({ startsAt: new Date(Date.now() - 3 * DAY_MS).toISOString(), endsAt: new Date(Date.now() - DAY_MS).toISOString() }),
    ).toEqual({ code: "discountCodeExpired" })
    expect(await check({ minimumType: "amount", minimumValue: 50 })).toEqual({
      code: "discountCodeMinimumNotMet",
      data: { minimumType: "amount", minimum: 50, missing: 30 },
    })
    expect(await check({ minimumType: "quantity", minimumValue: 3 })).toEqual({
      code: "discountCodeMinimumNotMet",
      data: { minimumType: "quantity", minimum: 3, missing: 2 },
    })
    expect(await check({ appliesTo: "products", productIds: [otherProduct] })).toEqual({ code: "discountCodeNotApplicable" })
    expect(await check({ oncePerCustomer: true })).toEqual({ code: "discountCodeNotEligible" })
    expect(
      await check({
        eligibility: "specificCustomers",
        customerFilter: { operator: "and", conditions: [{ field: "gender", operator: "eq", value: "female" }] },
      }),
    ).toEqual({ code: "discountCodeNotEligible" })
  })

  it("a contact identifier satisfies once-per-customer; an unknown contact is 404; 422 input", async () => {
    const { api } = await newTenant("p107-dc-validate-contact")
    const product = await createProduct(api, { price: 20 })
    const items = [{ productId: product, quantity: 1 }]
    const code = await create(api, { oncePerCustomer: true, discountValue: 50 })
    const contactId = await newContact(api)
    const ok = await api.post<Validation>("/v1/discount-codes/validate", { code: code.code, items, contact: idOf(contactId) })
    expect(ok.status, ok.text).toBe(200)
    expect(ok.json).toMatchObject({ valid: true, amount: 10, error: null })
    expectError(
      await api.post("/v1/discount-codes/validate", { code: code.code, items, contact: idOf(UNKNOWN_ID) }),
      404,
      "notFound",
      { message: "Contact not found" },
    )
    expectError(await api.post("/v1/discount-codes/validate", { code: code.code, items, contact: "nope" }), 404, "notFound")
    expectInvalid(await api.post("/v1/discount-codes/validate", { code: code.code, items: [] }), "items")
    expectInvalid(await api.post("/v1/discount-codes/validate", { code: "", items }), "code")
    expectInvalid(
      await api.post("/v1/discount-codes/validate", { code: code.code, items: [{ productId: product, quantity: 0 }] }),
      "items.0.quantity",
    )
    expectInvalid(await api.post("/v1/discount-codes/validate", { code: code.code, items, shippingType: "drone" }), "shippingType")
  })
})
