// placeCategories.list / get / create / update / delete and
// placeLocations.list / get / create / update / delete ("Places near me").
// Category names are unique per workspace ignoring case; deleting a category
// keeps its locations, uncategorized. Locations carry a Google Maps preview
// link built from the coordinates.

import { describe, expect, it } from "vitest"
import {
  type Api,
  expectError,
  expectInvalid,
  expectNotFound,
  ISO,
  keysOf,
  newTenant,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueName,
} from "./helpers.ts"

type Category = { id: string; name: string; rank: number; createdAt: string; updatedAt: string }
type Location = {
  id: string
  name: string
  latitude: number
  longitude: number
  categoryId: string | null
  categoryName: string | null
  previewUrl: string
  createdAt: string
  updatedAt: string
}

const createCategory = async (api: Api, body: { name?: string; rank?: number } = {}): Promise<Category> => {
  const response = await api.post<Category>("/v1/place-categories", { name: uniqueName("Cafe"), ...body })
  expect(response.status, response.text).toBe(200)
  return response.json
}

const createLocation = async (api: Api, body: Record<string, unknown> = {}): Promise<Location> => {
  const response = await api.post<Location>("/v1/place-locations", {
    name: uniqueName("Shop"),
    latitude: 10.7769,
    longitude: 106.7009,
    ...body,
  })
  expect(response.status, response.text).toBe(200)
  return response.json
}

const preview = (latitude: number, longitude: number) =>
  `https://www.google.com/maps/search/?api=1&query=${latitude}%2C${longitude}`

describe("placeCategories.create / get / list", () => {
  it("creates with rank 10 by default; lists by rank, name, id with location counts", async () => {
    const { api } = await newTenant("p113-pc-create")
    const response = await api.post<Category>("/v1/place-categories", { name: "  Coffee  " })
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(["createdAt", "id", "name", "rank", "updatedAt"])
    expect(response.json).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      name: "Coffee",
      rank: 10,
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
    })
    const coffee = response.json
    const bakery = await createCategory(api, { name: "Bakery", rank: 10 })
    const top = await createCategory(api, { name: "Zoo", rank: 1 })
    await createLocation(api, { categoryId: coffee.id })
    await createLocation(api, { categoryId: coffee.id })
    await createLocation(api)

    const listed = await api.get<{ data: Array<Category & { locationCount: number }> }>("/v1/place-categories")
    expect(listed.status, listed.text).toBe(200)
    expect(keysOf(listed.json)).toEqual(["data"])
    expect(listed.json.data).toEqual([
      { ...top, locationCount: 0 },
      { ...bakery, locationCount: 0 },
      { ...coffee, locationCount: 2 },
    ])
    const got = await api.get<Category>(`/v1/place-categories/${coffee.id}`)
    expect(got.status, got.text).toBe(200)
    expect(got.json).toEqual(coffee)
  })

  it("409 for a name already used (any case); 422 for a bad rank or name; 404 for unknown and foreign ids", async () => {
    const { api } = await newTenant("p113-pc-dup")
    const { api: other } = await newTenant("p113-pc-dup-other")
    const category = await createCategory(api, { name: "Pharmacy" })
    expectError(await api.post("/v1/place-categories", { name: "pharmacy" }), 409, "placeCategoryDuplicated", {
      defined: true,
      message: "Place category already exists",
    })
    expect((await other.post("/v1/place-categories", { name: "Pharmacy" })).status).toBe(200)
    expectInvalid(await api.post("/v1/place-categories", { name: "   " }), "name")
    expectInvalid(await api.post("/v1/place-categories", { name: "x", rank: 0 }), "rank")
    expectInvalid(await api.post("/v1/place-categories", { name: "x", rank: 2_147_483_648 }), "rank")
    expectInvalid(await api.post("/v1/place-categories", { name: "x", rank: 1.5 }), "rank")
    expectNotFound(await other.get(`/v1/place-categories/${category.id}`), "Place category not found")
    expectNotFound(await api.get(`/v1/place-categories/${UNKNOWN_ID}`), "Place category not found")
  })
})

describe("placeCategories.update / delete", () => {
  it("update changes the sent fields; needs one; 409 on a taken name; 404 unknown", async () => {
    const { api } = await newTenant("p113-pc-update")
    const category = await createCategory(api, { name: "Gym" })
    const taken = await createCategory(api, { name: "Pool" })
    const response = await api.patch<Category>(`/v1/place-categories/${category.id}`, { name: " Fitness ", rank: 3 })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ ...category, name: "Fitness", rank: 3, updatedAt: expect.stringMatching(ISO) })
    const rankOnly = await api.patch<Category>(`/v1/place-categories/${category.id}`, { rank: 4 })
    expect(rankOnly.json).toMatchObject({ name: "Fitness", rank: 4 })
    expectError(await api.patch(`/v1/place-categories/${category.id}`, { name: "POOL" }), 409, "placeCategoryDuplicated")
    expectInvalid(await api.patch(`/v1/place-categories/${category.id}`, {}), "")
    expectNotFound(await api.patch(`/v1/place-categories/${UNKNOWN_ID}`, { rank: 1 }), "Place category not found")
    expect((await api.get<Category>(`/v1/place-categories/${taken.id}`)).json.name).toBe("Pool")
  })

  it("delete returns how many locations became uncategorized and keeps them", async () => {
    const { api } = await newTenant("p113-pc-delete")
    const category = await createCategory(api)
    const first = await createLocation(api, { categoryId: category.id })
    const second = await createLocation(api, { categoryId: category.id })
    const response = await api.delete<{ uncategorizedLocationCount: number }>(`/v1/place-categories/${category.id}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ uncategorizedLocationCount: 2 })
    for (const location of [first, second]) {
      const got = await api.get<Location>(`/v1/place-locations/${location.id}`)
      expect(got.json).toMatchObject({ id: location.id, categoryId: null, categoryName: null })
    }
    expectNotFound(await api.delete(`/v1/place-categories/${category.id}`), "Place category not found")
    const empty = await createCategory(api)
    expect((await api.delete(`/v1/place-categories/${empty.id}`)).json).toEqual({ uncategorizedLocationCount: 0 })
  })
})

describe("placeLocations.create / get", () => {
  it("creates with the category name and a Google Maps preview link", async () => {
    const { api } = await newTenant("p113-loc-create")
    const category = await createCategory(api, { name: "Bank" })
    const response = await api.post<Location>("/v1/place-locations", {
      name: "  Head office  ",
      latitude: -33.8688,
      longitude: 151.2093,
      categoryId: category.id,
    })
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual([
      "categoryId",
      "categoryName",
      "createdAt",
      "id",
      "latitude",
      "longitude",
      "name",
      "previewUrl",
      "updatedAt",
    ])
    expect(response.json).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      name: "Head office",
      latitude: -33.8688,
      longitude: 151.2093,
      categoryId: category.id,
      categoryName: "Bank",
      previewUrl: preview(-33.8688, 151.2093),
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
    })
    const got = await api.get<Location>(`/v1/place-locations/${response.json.id}`)
    expect(got.json).toEqual(response.json)
    const plain = await createLocation(api, { latitude: 0, longitude: 0, categoryId: null })
    expect(plain).toMatchObject({ categoryId: null, categoryName: null, previewUrl: preview(0, 0) })
  })

  it("422 for out-of-range coordinates or a blank name; 404 for an unknown or foreign category or id", async () => {
    const { api } = await newTenant("p113-loc-invalid")
    const { api: other } = await newTenant("p113-loc-invalid-other")
    const foreignCategory = await createCategory(other)
    const base = { name: "x", latitude: 1, longitude: 1 }
    expectInvalid(await api.post("/v1/place-locations", { ...base, latitude: 91 }), "latitude")
    expectInvalid(await api.post("/v1/place-locations", { ...base, longitude: -181 }), "longitude")
    expectInvalid(await api.post("/v1/place-locations", { ...base, name: " " }), "name")
    expectInvalid(await api.post("/v1/place-locations", { name: "x", latitude: 1 }), "longitude")
    expectNotFound(
      await api.post("/v1/place-locations", { ...base, categoryId: foreignCategory.id }),
      "Place category does not exist.",
    )
    expectNotFound(await api.post("/v1/place-locations", { ...base, categoryId: UNKNOWN_ID }), "Place category does not exist.")
    const foreign = await createLocation(other)
    expectNotFound(await api.get(`/v1/place-locations/${foreign.id}`), "Place location not found")
    expectNotFound(await api.get(`/v1/place-locations/${UNKNOWN_ID}`), "Place location not found")
  })
})

describe("placeLocations.list", () => {
  it("is newest first; without paging it returns every row (pageCount 1); filters by name", async () => {
    const { api } = await newTenant("p113-loc-list")
    const { api: other } = await newTenant("p113-loc-list-other")
    expect((await api.get("/v1/place-locations")).json).toEqual({ data: [], pageCount: 1 })
    const token = uniqueName("Mall")
    const created: Location[] = []
    for (let index = 0; index < 3; index += 1) {
      created.push(await createLocation(api, { name: `${token} ${index}` }))
    }
    const solo = await createLocation(api, { name: uniqueName("Solo") })
    await createLocation(other)
    const all = await api.get<{ data: Location[]; pageCount: number }>("/v1/place-locations")
    expect(all.status, all.text).toBe(200)
    expect(all.json.pageCount).toBe(1)
    expect(all.json.data.map((location) => location.id)).toEqual([solo.id, created[2]?.id, created[1]?.id, created[0]?.id])
    expect(all.json.data[0]).toEqual(solo)

    const page1 = await api.get<{ data: Location[]; pageCount: number }>("/v1/place-locations", { query: { perPage: 3 } })
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data).toHaveLength(3)
    const page2 = await api.get<{ data: Location[] }>("/v1/place-locations", { query: { perPage: 3, page: 2 } })
    expect(page2.json.data.map((location) => location.id)).toEqual([created[0]?.id])
    const named = await api.get<{ data: Location[] }>("/v1/place-locations", { query: { name: token.toLowerCase() } })
    expect(named.json.data.map((location) => location.id)).toEqual([created[2]?.id, created[1]?.id, created[0]?.id])
    expectInvalid(await api.get("/v1/place-locations", { query: { perPage: 51 } }), "perPage")
  })
})

describe("placeLocations.update / delete", () => {
  it("update changes the sent fields and refreshes the preview link; needs one field", async () => {
    const { api } = await newTenant("p113-loc-update")
    const category = await createCategory(api, { name: "Hotel" })
    const location = await createLocation(api)
    const response = await api.patch<Location>(`/v1/place-locations/${location.id}`, {
      latitude: 48.8566,
      longitude: 2.3522,
      categoryId: category.id,
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      ...location,
      latitude: 48.8566,
      longitude: 2.3522,
      categoryId: category.id,
      categoryName: "Hotel",
      previewUrl: preview(48.8566, 2.3522),
      updatedAt: expect.stringMatching(ISO),
    })
    const cleared = await api.patch<Location>(`/v1/place-locations/${location.id}`, { categoryId: null, name: "Renamed" })
    expect(cleared.json).toMatchObject({ name: "Renamed", categoryId: null, categoryName: null })
    expectInvalid(await api.patch(`/v1/place-locations/${location.id}`, {}), "")
    expectNotFound(await api.patch(`/v1/place-locations/${UNKNOWN_ID}`, { name: "x" }), "Place location not found")
    expectNotFound(
      await api.patch(`/v1/place-locations/${location.id}`, { categoryId: UNKNOWN_ID }),
      "Place category does not exist.",
    )
  })

  it("delete is 204 and 404s afterwards and for foreign ids", async () => {
    const { api } = await newTenant("p113-loc-delete")
    const { api: other } = await newTenant("p113-loc-delete-other")
    const location = await createLocation(api)
    const foreign = await createLocation(other)
    const response = await api.delete(`/v1/place-locations/${location.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectNotFound(await api.get(`/v1/place-locations/${location.id}`), "Place location not found")
    expectNotFound(await api.delete(`/v1/place-locations/${location.id}`), "Place location not found")
    expectNotFound(await api.delete(`/v1/place-locations/${foreign.id}`), "Place location not found")
    expect((await other.get(`/v1/place-locations/${foreign.id}`)).status).toBe(200)
  })
})
