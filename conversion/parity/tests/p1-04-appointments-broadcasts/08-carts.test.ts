// carts.get

import { describe, expect, it } from "vitest"
import { Normalizer } from "../../src/normalize.ts"
import { createContact, createWebchat, errorShape, expectError, ext, newTenant } from "./helpers.ts"

type Cart = {
  items: unknown[]
  currency: string
  lines: unknown[]
  unavailable: Array<{ productId: string; name: string | null }>
  totals: Record<string, number>
  enteredDiscountCode: string | null
  [key: string]: unknown
}

describe("carts.get", () => {
  it("returns an empty cart for a contact without one, by id or email", async () => {
    const normalizer = new Normalizer()
    const { tenant, api } = await newTenant("cart-empty")
    const webchat = await createWebchat(api)
    const contact = await createContact(api, tenant, {
      inboxId: webchat.inboxId,
      firstName: "Ada",
      email: "ada-cart@parity.test",
    })
    const byId = await api.get<Cart>(`/v1/carts/id:${contact.contactId}`)
    expect(byId.status, byId.text).toBe(200)
    expect(byId.json).toMatchObject({ items: [], lines: [], unavailable: [], enteredDiscountCode: null })
    expect(byId.json.totals).toMatchObject({ subtotal: 0, total: 0 })
    expect(normalizer.normalize(byId.json)).toMatchSnapshot("empty cart")

    const byEmail = await api.get<Cart>(`/v1/carts/${encodeURIComponent("email:ada-cart@parity.test")}`)
    expect(byEmail.status, byEmail.text).toBe(200)
    expect(byEmail.json).toEqual(byId.json)
  })

  it("lists items whose product no longer exists as unavailable, out of the totals", async () => {
    const normalizer = new Normalizer()
    const { tenant, api } = await newTenant("cart-items")
    const webchat = await createWebchat(api)
    const contact = await createContact(api, tenant, { inboxId: webchat.inboxId, firstName: "Ada" })
    await (await ext()).createCart({
      workspaceId: tenant.workspaceId,
      contactId: contact.contactId,
      items: [{ productId: "123456789012345678", variantId: null, quantity: 2 }],
    })
    const response = await api.get<Cart>(`/v1/carts/id:${contact.contactId}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json.items).toEqual([{ productId: "123456789012345678", variantId: null, quantity: 2 }])
    expect(response.json.lines).toEqual([])
    expect(response.json.unavailable).toEqual([
      expect.objectContaining({ productId: "123456789012345678", quantity: 2, name: null }),
    ])
    expect(response.json.totals).toMatchObject({ subtotal: 0, total: 0 })
    expect(normalizer.normalize(response.json, { keep: ["productId"] })).toMatchSnapshot("cart with unavailable item")
  })

  it("is 404 for an unknown or foreign contact and rejects a bare identifier", async () => {
    const { api: other, tenant: otherTenant } = await newTenant("cart-other")
    const otherWebchat = await createWebchat(other)
    const foreign = await createContact(other, otherTenant, { inboxId: otherWebchat.inboxId, firstName: "Eve" })
    const { api } = await newTenant("cart-404")
    const unknown = await api.get("/v1/carts/id:1")
    const body = expectError(unknown, 404, "notFound")
    expect(body.message).toBe("Contact not found")
    expect(errorShape(unknown)).toMatchSnapshot("404 shape")
    expectError(await api.get(`/v1/carts/id:${foreign.contactId}`), 404, "notFound")
    expectError(await api.get(`/v1/carts/${encodeURIComponent("email:nobody@parity.test")}`), 404, "notFound")

    const bare = await api.get("/v1/carts/12345")
    expect(bare.status).toBeGreaterThanOrEqual(400)
    expect(errorShape(bare)).toMatchSnapshot("bare identifier")
  })

  it("rejects a blank identifier and an unknown prefix", async () => {
    const { api } = await newTenant("cart-invalid")
    const blank = await api.get("/v1/carts/%20")
    expect(blank.status).toBeGreaterThanOrEqual(400)
    expect(errorShape(blank)).toMatchSnapshot("blank identifier")
    const prefix = await api.get("/v1/carts/name:Ada")
    expect(prefix.status).toBeGreaterThanOrEqual(400)
    expect(errorShape(prefix)).toMatchSnapshot("unknown prefix")
  })
})
