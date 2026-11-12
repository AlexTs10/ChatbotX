// productCategories.list / create / update (PUT) / setVisible / delete and
// productVendors.list / create / update / delete. Categories form two levels
// (category -> sub-category); names are unique per parent (exact case).
// Vendor names are unique per workspace ignoring case.

import { describe, expect, it } from "vitest"
import {
  type Api,
  createProduct,
  expectError,
  expectInvalid,
  expectNotFound,
  keysOf,
  newTenant,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueName,
} from "./helpers.ts"

type Category = { id: string; parentId: string | null; name: string; imageUrl: string | null; isVisible: boolean; rank: number }
type Vendor = {
  id: string
  name: string
  shortDescription: string | null
  phone: string | null
  storeAddress: string | null
  imageUrl: string | null
  isActive: boolean
  discountPercent: number
  minimumOrderValue: number
  openHoursId: string | null
  categoryId: string | null
  subcategoryId: string | null
  rank: number
}

const CATEGORY_KEYS = ["id", "imageUrl", "isVisible", "name", "parentId", "rank"]

const createCategory = async (api: Api, body: Record<string, unknown> = {}): Promise<Category> => {
  const response = await api.post<Category>("/v1/product-categories", { name: uniqueName("Cat"), ...body })
  expect(response.status, response.text).toBe(200)
  return response.json
}

const listCategories = async (api: Api, query: Record<string, string> = {}) => {
  const response = await api.get<{ data: Array<Category & { productCount: number }> }>("/v1/product-categories", { query })
  expect(response.status, response.text).toBe(200)
  expect(keysOf(response.json)).toEqual(["data"])
  return response.json.data
}

const createVendor = async (api: Api, body: Record<string, unknown> = {}): Promise<Vendor> => {
  const response = await api.post<Vendor>("/v1/product-vendors", { name: uniqueName("Vendor"), ...body })
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("productCategories.create / list", () => {
  it("creates top-level and sub-categories (rank 10, visible by default); lists by rank then name with product counts", async () => {
    const { api } = await newTenant("p114-pc-create")
    const response = await api.post<Category>("/v1/product-categories", { name: "  Drinks  " })
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(CATEGORY_KEYS)
    const drinks = response.json
    expect(drinks).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      parentId: null,
      name: "Drinks",
      imageUrl: null,
      isVisible: true,
      rank: 10,
    })
    const tea = await createCategory(api, {
      name: "Tea",
      parentId: drinks.id,
      imageUrl: "https://example.com/tea.png",
      isVisible: false,
      rank: 2,
    })
    expect(tea).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      parentId: drinks.id,
      name: "Tea",
      imageUrl: "https://example.com/tea.png",
      isVisible: false,
      rank: 2,
    })
    const apparel = await createCategory(api, { name: "Apparel" })
    await createProduct(api, { categoryId: drinks.id })
    await createProduct(api, { categoryId: drinks.id, subcategoryId: tea.id })

    expect(await listCategories(api)).toEqual([
      { ...tea, productCount: 1 },
      { ...apparel, productCount: 0 },
      { ...drinks, productCount: 2 },
    ])
    expect(await listCategories(api, { parentId: drinks.id })).toEqual([{ ...tea, productCount: 1 }])
    expect((await listCategories(api, { name: "APP" })).map((category) => category.id)).toEqual([apparel.id])
  })

  it("refuses a duplicate name under the same parent, a third level, an unknown parent and bad input", async () => {
    const { api } = await newTenant("p114-pc-refused")
    const parent = await createCategory(api, { name: "Shoes" })
    const child = await createCategory(api, { name: "Kids", parentId: parent.id })
    // Same name under another parent is fine; a different case is another name.
    expect((await api.post("/v1/product-categories", { name: "Kids" })).status).toBe(200)
    expect((await api.post("/v1/product-categories", { name: "shoes" })).status).toBe(200)
    expectError(await api.post("/v1/product-categories", { name: "Shoes" }), 400, "productCategoryDuplicated", {
      message: "Product category already exists",
    })
    expectError(
      await api.post("/v1/product-categories", { name: "Toddlers", parentId: child.id }),
      400,
      "productCategoryNestingTooDeep",
      { message: "A sub-category cannot contain further sub-categories" },
    )
    expectNotFound(
      await api.post("/v1/product-categories", { name: "Orphan", parentId: UNKNOWN_ID }),
      "Parent product category not found",
      false,
    )
    expectInvalid(await api.post("/v1/product-categories", { name: "  " }), "name")
    expectInvalid(await api.post("/v1/product-categories", { name: "x", imageUrl: "not-a-url" }), "imageUrl")
    expectInvalid(await api.post("/v1/product-categories", { name: "x", rank: 1.5 }), "rank")
  })
})

describe("productCategories.update / setVisible / delete", () => {
  it("PUT renames and keeps the parent when parentId is omitted; null promotes; an id files it under that parent", async () => {
    const { api } = await newTenant("p114-pc-update")
    const parent = await createCategory(api, { name: "Food" })
    const other = await createCategory(api, { name: "Snacks" })
    const child = await createCategory(api, { name: "Fruit", parentId: parent.id })
    const renamed = await api.put<Category>(`/v1/product-categories/${child.id}`, { name: "Fresh fruit", rank: 3 })
    expect(renamed.status, renamed.text).toBe(200)
    expect(renamed.json).toEqual({ ...child, name: "Fresh fruit", rank: 3 })
    const promoted = await api.put<Category>(`/v1/product-categories/${child.id}`, { name: "Fresh fruit", parentId: null })
    expect(promoted.json.parentId).toBeNull()
    const moved = await api.put<Category>(`/v1/product-categories/${child.id}`, { name: "Fresh fruit", parentId: other.id })
    expect(moved.json.parentId).toBe(other.id)

    expectError(
      await api.put(`/v1/product-categories/${parent.id}`, { name: "Food", parentId: parent.id }),
      400,
      "productCategoryParentSelf",
      { message: "A product category cannot be its own parent" },
    )
    expectError(
      await api.put(`/v1/product-categories/${other.id}`, { name: "Snacks", parentId: parent.id }),
      400,
      "productCategoryNestingTooDeep",
      { message: "A category with sub-categories cannot itself become a sub-category" },
    )
    expectError(await api.put(`/v1/product-categories/${other.id}`, { name: "Food" }), 400, "productCategoryDuplicated")
    expectNotFound(await api.put(`/v1/product-categories/${UNKNOWN_ID}`, { name: "x" }), "Product category not found")
    expectInvalid(await api.put(`/v1/product-categories/${other.id}`, { rank: 1 }), "name")
  })

  it("setVisible toggles only isVisible", async () => {
    const { api } = await newTenant("p114-pc-visible")
    const category = await createCategory(api)
    const hidden = await api.patch<Category>(`/v1/product-categories/${category.id}/visible`, { isVisible: false })
    expect(hidden.status, hidden.text).toBe(200)
    expect(hidden.json).toEqual({ ...category, isVisible: false })
    const shown = await api.patch<Category>(`/v1/product-categories/${category.id}/visible`, { isVisible: true })
    expect(shown.json.isVisible).toBe(true)
    expectNotFound(
      await api.patch(`/v1/product-categories/${UNKNOWN_ID}/visible`, { isVisible: true }),
      "Product category not found",
    )
    expectInvalid(await api.patch(`/v1/product-categories/${category.id}/visible`, {}), "isVisible")
  })

  it("delete is 204, removes its sub-categories and leaves its products uncategorized", async () => {
    const { api } = await newTenant("p114-pc-delete")
    const parent = await createCategory(api)
    const child = await createCategory(api, { parentId: parent.id })
    const product = await createProduct(api, { categoryId: parent.id, subcategoryId: child.id })
    const response = await api.delete(`/v1/product-categories/${parent.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expect(await listCategories(api)).toEqual([])
    const got = await api.get<{ categoryId: string | null; subcategoryId: string | null }>(`/v1/products/${product.id}`)
    expect(got.json).toMatchObject({ categoryId: null, subcategoryId: null })
    expectNotFound(await api.delete(`/v1/product-categories/${parent.id}`), "Product category not found")
  })
})

describe("productVendors", () => {
  it("create fills the defaults; list is by rank then name with product counts and pages", async () => {
    const { api } = await newTenant("p114-pv-create")
    const response = await api.post<Vendor>("/v1/product-vendors", { name: "  Acme  " })
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual([
      "categoryId",
      "discountPercent",
      "id",
      "imageUrl",
      "isActive",
      "minimumOrderValue",
      "name",
      "openHoursId",
      "phone",
      "rank",
      "shortDescription",
      "storeAddress",
      "subcategoryId",
    ])
    const acme = response.json
    expect(acme).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      name: "Acme",
      shortDescription: null,
      phone: null,
      storeAddress: null,
      imageUrl: null,
      isActive: true,
      discountPercent: 0,
      minimumOrderValue: 0,
      openHoursId: null,
      categoryId: null,
      subcategoryId: null,
      rank: 10,
    })
    const category = await createCategory(api)
    const full = await createVendor(api, {
      name: "Bolt",
      shortDescription: "Tools",
      phone: "+15550001",
      storeAddress: "1 Road",
      imageUrl: "https://example.com/bolt.png",
      isActive: false,
      discountPercent: 12.5,
      minimumOrderValue: 20,
      categoryId: category.id,
      rank: 1,
    })
    expect(full).toMatchObject({ name: "Bolt", discountPercent: 12.5, minimumOrderValue: 20, isActive: false, categoryId: category.id })
    const zed = await createVendor(api, { name: "Zed" })
    await createProduct(api, { vendorId: acme.id })

    const all = await api.get<{ data: Array<Vendor & { productCount: number }>; pageCount: number }>("/v1/product-vendors")
    expect(all.status, all.text).toBe(200)
    expect(all.json).toEqual({
      data: [
        { ...full, productCount: 0 },
        { ...acme, productCount: 1 },
        { ...zed, productCount: 0 },
      ],
      pageCount: 1,
    })
    const page2 = await api.get<{ data: Vendor[]; pageCount: number }>("/v1/product-vendors", { query: { perPage: 2, page: 2 } })
    expect(page2.json.pageCount).toBe(2)
    expect(page2.json.data.map((vendor) => vendor.id)).toEqual([zed.id])
    const named = await api.get<{ data: Vendor[] }>("/v1/product-vendors", { query: { name: "acm" } })
    expect(named.json.data.map((vendor) => vendor.id)).toEqual([acme.id])
  })

  it("400 for a duplicate name (any case); 404 for an unknown category; 422 bad input", async () => {
    const { api } = await newTenant("p114-pv-refused")
    await createVendor(api, { name: "Globex" })
    expectError(await api.post("/v1/product-vendors", { name: "GLOBEX" }), 400, "productVendorDuplicated", {
      defined: true,
      message: "Vendor already exists",
    })
    expectNotFound(
      await api.post("/v1/product-vendors", { name: "x", categoryId: UNKNOWN_ID }),
      "Product category does not exist.",
      false,
    )
    expectInvalid(await api.post("/v1/product-vendors", { name: "x", discountPercent: 101 }), "discountPercent")
    expectInvalid(await api.post("/v1/product-vendors", { name: "x", minimumOrderValue: -1 }), "minimumOrderValue")
    expectInvalid(await api.post("/v1/product-vendors", { name: "" }), "name")
  })

  it("update changes the sent fields; delete is 204 and unlinks its products", async () => {
    const { api } = await newTenant("p114-pv-update")
    const vendor = await createVendor(api, { name: "Initech" })
    const taken = await createVendor(api, { name: "Hooli" })
    const updated = await api.patch<Vendor>(`/v1/product-vendors/${vendor.id}`, { phone: "123", isActive: false })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).toEqual({ ...vendor, phone: "123", isActive: false })
    expectError(await api.patch(`/v1/product-vendors/${vendor.id}`, { name: "hooli" }), 400, "productVendorDuplicated")
    expectNotFound(await api.patch(`/v1/product-vendors/${UNKNOWN_ID}`, { name: "x" }), "Vendor not found")

    const product = await createProduct(api, { vendorId: vendor.id })
    expect(product.vendorId).toBe(vendor.id)
    const deleted = await api.delete(`/v1/product-vendors/${vendor.id}`)
    expect(deleted.status, deleted.text).toBe(204)
    expect((await api.get<{ vendorId: string | null }>(`/v1/products/${product.id}`)).json.vendorId).toBeNull()
    expectNotFound(await api.delete(`/v1/product-vendors/${vendor.id}`), "Vendor not found")
    expect((await api.get<{ data: Vendor[] }>("/v1/product-vendors")).json.data.map((v) => v.id)).toEqual([taken.id])
  })
})
