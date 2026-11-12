// products.create / get / list / update (PUT) / setActive / delete /
// deleteMany / listSendable. `products.list` has no ORDER BY in the original
// (and no public `sort`), so lists are compared as sets. `update` is a full
// replace: omitted fields fall back to the create defaults.

import { describe, expect, it } from "vitest"
import {
  type Api,
  createProduct,
  ext,
  expectInvalid,
  expectNotFound,
  ISO,
  keysOf,
  newTenant,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueName,
} from "./helpers.ts"

const PRODUCT_KEYS = [
  "allowOutOfStockPurchase",
  "allowSpecialRequest",
  "categoryId",
  "createdAt",
  "currency",
  "discount",
  "externalHandle",
  "externalId",
  "id",
  "images",
  "inventoryPolicy",
  "inventoryQuantity",
  "isActive",
  "isAddonOnly",
  "isSearchable",
  "longDescription",
  "name",
  "price",
  "productUrl",
  "rank",
  "shortDescription",
  "sku",
  "source",
  "status",
  "subcategoryId",
  "tags",
  "taxes",
  "updatedAt",
  "vendor",
  "vendorId",
  "workspaceId",
]

type Product = Record<string, unknown> & { id: string; name: string }

const get = async (api: Api, id: string): Promise<Product> => {
  const response = await api.get<Product>(`/v1/products/${id}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

const listIds = async (api: Api, query: Record<string, string | number> = {}): Promise<string[]> => {
  const response = await api.get<{ data: Product[]; pageCount: number }>("/v1/products", { query })
  expect(response.status, response.text).toBe(200)
  return response.json.data.map((product) => product.id)
}

describe("products.create / get", () => {
  it("creates a product with the defaults; get adds variants and addons", async () => {
    const { tenant, api } = await newTenant("p114-pr-create")
    const response = await api.post<Product>("/v1/products", { name: "  Espresso  ", price: 2.5 })
    expect(response.status, response.text).toBe(200)
    const product = response.json
    expect(keysOf(product)).toEqual(PRODUCT_KEYS)
    expect(product).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      workspaceId: tenant.workspaceId,
      name: "Espresso",
      shortDescription: "",
      longDescription: "",
      price: 2.5,
      taxes: 0,
      discount: 0,
      currency: "USD",
      productUrl: "",
      sku: "",
      inventoryPolicy: "dont_track",
      inventoryQuantity: 0,
      allowOutOfStockPurchase: false,
      images: [],
      tags: [],
      vendor: null,
      vendorId: null,
      rank: 10,
      categoryId: null,
      subcategoryId: null,
      status: "active",
      isActive: true,
      isSearchable: true,
      allowSpecialRequest: false,
      isAddonOnly: false,
      source: "chatbotx",
      externalId: null,
      externalHandle: null,
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
    })
    const detail = await get(api, product.id)
    expect(detail).toEqual({ ...product, variantOptions: [], variants: [], addons: [] })
  })

  it("stores variants, addons, tags, images, vendor and category", async () => {
    const { api } = await newTenant("p114-pr-full")
    const category = await api.post<{ id: string }>("/v1/product-categories", { name: uniqueName("Cat") })
    const vendor = await api.post<{ id: string; name: string }>("/v1/product-vendors", { name: uniqueName("Vendor") })
    const addon = await createProduct(api, { name: "Extra shot", price: 0.5, isAddonOnly: true })
    const response = await api.post<Product>("/v1/products", {
      name: "Latte",
      price: 4,
      taxes: 10,
      discount: 5,
      sku: "LAT-1",
      tags: ["hot", "milk"],
      images: [{ mode: "link", url: "https://example.com/latte.png" }],
      inventoryPolicy: "track",
      inventoryQuantity: 7,
      categoryId: category.json.id,
      vendorId: vendor.json.id,
      variantOptions: [{ name: "Size", values: ["S", "L"], position: 0 }],
      variants: [
        { combination: { Size: "S" }, price: 4, isEnabled: true },
        { combination: { Size: "L" }, price: 5, isEnabled: false },
      ],
      addons: [{ name: "Extras", maxSelections: 2, addonProductIds: [addon.id] }],
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({
      name: "Latte",
      price: 4,
      discount: 5,
      sku: "LAT-1",
      tags: ["hot", "milk"],
      images: [{ url: "https://example.com/latte.png", type: "link" }],
      inventoryPolicy: "track",
      inventoryQuantity: 7,
      categoryId: category.json.id,
      vendorId: vendor.json.id,
      vendor: vendor.json.name,
    })
    const detail = await get(api, response.json.id)
    expect(detail.variantOptions).toEqual([{ name: "Size", values: ["S", "L"], position: 0 }])
    expect(detail.variants).toMatchObject([
      { combination: { Size: "S" }, price: 4, isEnabled: true },
      { combination: { Size: "L" }, price: 5, isEnabled: false },
    ])
    expect(detail.addons).toEqual([{ name: "Extras", maxSelections: 2, addonProductIds: [addon.id] }])
  })

  it("404 for an unknown category, sub-category, vendor or addon; 422 bad input; 404 unknown id", async () => {
    const { api } = await newTenant("p114-pr-refused")
    const { api: other } = await newTenant("p114-pr-refused-other")
    const create = (body: Record<string, unknown>) => api.post("/v1/products", { name: "x", ...body })
    expectNotFound(await create({ categoryId: UNKNOWN_ID }), "Product category does not exist.", false)
    expectNotFound(await create({ subcategoryId: UNKNOWN_ID }), "Product sub-category does not exist.", false)
    expectNotFound(await create({ vendorId: UNKNOWN_ID }), "Vendor does not exist.", false)
    expectNotFound(
      await create({ addons: [{ name: "A", maxSelections: 1, addonProductIds: [UNKNOWN_ID] }] }),
      "One or more addon products do not exist.",
      false,
    )
    expectInvalid(await create({ price: -1 }), "price")
    expectInvalid(await create({ discount: 101 }), "discount")
    expectInvalid(await create({ productUrl: "ftp://example.com" }), "productUrl")
    expectInvalid(await create({ inventoryPolicy: "always" }), "inventoryPolicy")
    const foreign = await createProduct(other)
    expectNotFound(await api.get(`/v1/products/${foreign.id}`), "Product does not exist.")
    expectNotFound(await api.get(`/v1/products/${UNKNOWN_ID}`), "Product does not exist.")
    expect(await listIds(api)).toEqual([])
  })
})

describe("products.list", () => {
  it("pages and filters by name and category (order not defined)", async () => {
    const { api } = await newTenant("p114-pr-list")
    const { api: other } = await newTenant("p114-pr-list-other")
    const category = await api.post<{ id: string }>("/v1/product-categories", { name: uniqueName("Cat") })
    const token = uniqueName("Mug")
    const ids: string[] = []
    for (let index = 0; index < 3; index += 1) {
      ids.push((await createProduct(api, { name: `${token} ${index}`, categoryId: index === 0 ? category.json.id : null })).id)
    }
    const solo = await createProduct(api, { name: uniqueName("Plate") })
    await createProduct(other)
    const all = await api.get<{ data: Product[]; pageCount: number }>("/v1/products")
    expect(all.status, all.text).toBe(200)
    expect(keysOf(all.json)).toEqual(["data", "pageCount"])
    expect(all.json.pageCount).toBe(1)
    expect(all.json.data.map((product) => product.id).sort()).toEqual([...ids, solo.id].sort())
    expect(keysOf(all.json.data[0])).toEqual(PRODUCT_KEYS)

    const page1 = await listIds(api, { perPage: 3 })
    const page2 = await listIds(api, { perPage: 3, page: 2 })
    expect(page1).toHaveLength(3)
    expect(page2).toHaveLength(1)
    expect([...page1, ...page2].sort()).toEqual([...ids, solo.id].sort())
    expect((await listIds(api, { name: token.toUpperCase() })).sort()).toEqual([...ids].sort())
    expect(await listIds(api, { categoryId: category.json.id })).toEqual([ids[0]])
    expectInvalid(await api.get("/v1/products", { query: { perPage: 51 } }), "perPage")
  })
})

describe("products.update / setActive", () => {
  it("PUT replaces the product; omitted fields fall back to the defaults; 204 with no body", async () => {
    const { api } = await newTenant("p114-pr-update")
    const product = await createProduct(api, { name: "Old", price: 9, sku: "OLD", tags: ["a"] })
    const response = await api.put(`/v1/products/${product.id}`, { name: "New", discount: 10 })
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    const after = await get(api, product.id)
    expect(after).toMatchObject({ id: product.id, name: "New", discount: 10, price: 0, sku: "", tags: [] })
    expectInvalid(await api.put(`/v1/products/${product.id}`, { price: 1 }), "name")
    expectNotFound(await api.put(`/v1/products/${UNKNOWN_ID}`, { name: "x" }), "Product does not exist.")
  })

  it("setActive switches the status between active and draft; 204", async () => {
    const { api } = await newTenant("p114-pr-active")
    const product = await createProduct(api)
    const off = await api.patch(`/v1/products/${product.id}/active`, { isActive: false })
    expect(off.status, off.text).toBe(204)
    expect(await get(api, product.id)).toMatchObject({ status: "draft", isActive: false })
    expect((await api.patch(`/v1/products/${product.id}/active`, { isActive: true })).status).toBe(204)
    expect(await get(api, product.id)).toMatchObject({ status: "active", isActive: true })
    expectNotFound(await api.patch(`/v1/products/${UNKNOWN_ID}/active`, { isActive: true }), "Product does not exist.")
    expectInvalid(await api.patch(`/v1/products/${product.id}/active`, {}), "isActive")
  })
})

describe("products.delete / deleteMany", () => {
  it("delete is 204 and 404s for unknown and foreign ids; deleteMany ignores them", async () => {
    const { api } = await newTenant("p114-pr-delete")
    const { api: other } = await newTenant("p114-pr-delete-other")
    const product = await createProduct(api)
    const foreign = await createProduct(other)
    const response = await api.delete(`/v1/products/${product.id}`)
    expect(response.status, response.text).toBe(204)
    expectNotFound(await api.get(`/v1/products/${product.id}`), "Product does not exist.")
    expectNotFound(await api.delete(`/v1/products/${product.id}`), "Product does not exist.")
    expectNotFound(await api.delete(`/v1/products/${foreign.id}`), "Product does not exist.")

    const first = await createProduct(api)
    const second = await createProduct(api)
    const kept = await createProduct(api)
    const bulk = await api.post("/v1/products/bulk-delete", { ids: [first.id, second.id, foreign.id, UNKNOWN_ID] })
    expect(bulk.status, bulk.text).toBe(204)
    expect(await listIds(api)).toEqual([kept.id])
    expect((await other.get(`/v1/products/${foreign.id}`)).status).toBe(200)
    expectInvalid(await api.post("/v1/products/bulk-delete", { ids: ["abc"] }), "ids.0")
  })
})

describe("products.listSendable", () => {
  it("is not ready for a channel without product messages (messenger) or without a catalog; ready once a catalog is bound", async () => {
    const { tenant, api } = await newTenant("p114-pr-sendable")
    // Only WhatsApp sends product messages (from a Meta catalog).
    const unsupported = await api.get("/v1/products/sendable", { query: { channel: "messenger" } })
    expect(unsupported.status, unsupported.text).toBe(200)
    expect(unsupported.json).toEqual({ ready: false, reason: "channelUnsupported", data: [], pageCount: 0 })
    const noCatalog = await api.get("/v1/products/sendable", { query: { channel: "whatsapp" } })
    expect(noCatalog.json).toEqual({ ready: false, reason: "catalogNotConnected", data: [], pageCount: 0 })

    await (await ext()).createMetaCatalogConnection({
      workspaceId: tenant.workspaceId,
      accessToken: `mc-token-${uniqueName("t")}`,
      catalogId: `${Date.now()}1`,
      catalogName: "Parity catalog",
      businessId: `${Date.now()}2`,
    })
    const ready = await api.get("/v1/products/sendable", { query: { channel: "whatsapp" } })
    expect(ready.json).toEqual({ ready: true, data: [], pageCount: 0 })
    expectInvalid(await api.get("/v1/products/sendable"), "channel")
    expectInvalid(await api.get("/v1/products/sendable", { query: { channel: "fax" } }), "channel")
  })
})
