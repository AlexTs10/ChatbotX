// products.getMetaCatalog / listMetaCatalogBusinesses / createMetaCatalog /
// selectMetaCatalog / syncMetaCatalog / searchMetaCatalogItems /
// disconnectMetaCatalog. The Meta Catalog OAuth connect has no public API
// path: the connection comes from the P1.12 fixture extension (encrypted
// token). Graph calls go to the fake upstream with a per-test token; the
// shared `/me/businesses` path answers once per script (`times: 1`).

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import {
  type Api,
  ext,
  expectInvalid,
  expectNotFound,
  expectValidation,
  GRAPH,
  graphPath,
  ISO,
  keysOf,
  newTenant,
  uniqueDigits,
  uniqueName,
  upstreamCalls,
  wait,
} from "./helpers.ts"

const CONNECTION_KEYS = [
  "authMode",
  "businessId",
  "catalogId",
  "catalogName",
  "createdAt",
  "currency",
  "id",
  "importError",
  "importFailedCount",
  "importStatus",
  "importTotalCount",
  "importedCount",
  "lastImportedAt",
  "status",
  "storeUrl",
  "tokenExpiresAt",
  "updatedAt",
]

type Connection = Record<string, unknown> & { id: string; catalogId: string | null; importStatus: string }
type Run = Record<string, unknown> & { id: string; status: string; direction: string }
type State = { connection: Connection | null; history: Run[] }

const connect = async (tenant: Tenant, extra: { catalogId?: string; catalogName?: string; businessId?: string } = {}) => {
  const accessToken = `mc-${uniqueName("token")}`
  const created = await (await ext()).createMetaCatalogConnection({ workspaceId: tenant.workspaceId, accessToken, ...extra })
  return { accessToken, ...created }
}

const state = async (api: Api): Promise<State> => {
  const response = await api.get<State>("/v1/products/meta-catalog")
  expect(response.status, response.text).toBe(200)
  expect(keysOf(response.json)).toEqual(["connection", "history"])
  return response.json
}

describe("products.getMetaCatalog / searchMetaCatalogItems without a connection", () => {
  it("reports no connection; the Graph routes 404", async () => {
    const { api } = await newTenant("p114-mc-none")
    expect(await state(api)).toEqual({ connection: null, history: [] })
    const search = await api.get("/v1/products/meta-catalog/items")
    expect(search.status, search.text).toBe(200)
    expect(search.json).toEqual({ connected: false, items: [] })
    const message = "Meta Catalog integration not found"
    expectNotFound(await api.get("/v1/products/meta-catalog/businesses"), message)
    expectNotFound(await api.post("/v1/products/meta-catalog", { businessId: "123", name: "x" }), message)
    expectNotFound(await api.post("/v1/products/meta-catalog/select", { catalogId: "123" }), message)
    expectNotFound(await api.post("/v1/products/meta-catalog/sync", { scope: "all" }), message)
    const disconnected = await api.delete("/v1/products/meta-catalog")
    expect(disconnected.status, disconnected.text).toBe(204)
  })
})

describe("products.getMetaCatalog / listMetaCatalogBusinesses", () => {
  it("shows the connection without its token and lists the token's businesses", async () => {
    const { tenant, api } = await newTenant("p114-mc-businesses")
    const { accessToken, connectionId } = await connect(tenant)
    const current = await state(api)
    expect(keysOf(current.connection)).toEqual(CONNECTION_KEYS)
    expect(current).toEqual({
      connection: {
        id: connectionId,
        catalogId: null,
        catalogName: null,
        businessId: null,
        authMode: "oauth",
        tokenExpiresAt: null,
        status: "active",
        importStatus: "idle",
        importTotalCount: 0,
        importedCount: 0,
        importFailedCount: 0,
        importError: null,
        lastImportedAt: null,
        currency: "VND",
        storeUrl: null,
        createdAt: expect.stringMatching(ISO),
        updatedAt: expect.stringMatching(ISO),
      },
      history: [],
    })
    expect(JSON.stringify(current)).not.toContain(accessToken)

    await fake.script({
      match: { host: GRAPH, method: "GET", pathRegex: graphPath("me/businesses") },
      respond: { status: 200, json: { data: [{ id: "111", name: "Parity Biz" }, { id: "222" }, { name: "no id" }] } },
      times: 1,
    })
    const since = await fake.cursor()
    const response = await api.get("/v1/products/meta-catalog/businesses")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual([{ id: "111", name: "Parity Biz" }, { id: "222" }])
    const calls = await upstreamCalls({ host: GRAPH, since, bearer: accessToken })
    expect(calls).toHaveLength(1)
    expect(calls[0]?.path).toMatch(/^\/v23\.0\/me\/businesses$/)
    expect(calls[0]?.query).toMatchObject({ fields: "id,name", limit: "100" })
  })
})

describe("products.createMetaCatalog", () => {
  it("creates a commerce catalog in the business and binds it", async () => {
    const { tenant, api } = await newTenant("p114-mc-create")
    const { accessToken, connectionId } = await connect(tenant)
    const businessId = uniqueDigits()
    const catalogId = uniqueDigits()
    await fake.script({
      match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${businessId}/owned_product_catalogs`) },
      respond: { status: 200, json: { id: catalogId } },
      times: 1,
    })
    const since = await fake.cursor()
    const response = await api.post<Connection>("/v1/products/meta-catalog", { businessId, name: "  Parity catalog  " })
    expect(response.status, response.text).toBe(201)
    expect(keysOf(response.json)).toEqual(CONNECTION_KEYS)
    expect(response.json).toMatchObject({ id: connectionId, catalogId, catalogName: "Parity catalog", businessId })
    const calls = await upstreamCalls({ host: GRAPH, since, bearer: accessToken })
    expect(calls).toHaveLength(1)
    expect(Object.fromEntries(new URLSearchParams(calls[0]?.bodyText ?? ""))).toEqual({
      name: "Parity catalog",
      vertical: "commerce",
    })
    expect((await state(api)).connection).toMatchObject({ catalogId, catalogName: "Parity catalog", businessId })
    const search = await api.get("/v1/products/meta-catalog/items", { query: { keyword: "mug" } })
    expect(search.json).toEqual({ connected: true, items: [] })

    expectInvalid(await api.post("/v1/products/meta-catalog", { businessId: "abc", name: "x" }), "businessId")
    expectInvalid(await api.post("/v1/products/meta-catalog", { businessId: "1", name: "" }), "name")
  })
})

describe("products.selectMetaCatalog", () => {
  it("binds an existing catalog and imports its products in the worker (202)", async () => {
    const { tenant, api } = await newTenant("p114-mc-select")
    const { accessToken, connectionId } = await connect(tenant)
    const catalogId = uniqueDigits()
    const businessId = uniqueDigits()
    await fake.script(
      {
        match: { host: GRAPH, method: "GET", pathRegex: graphPath(catalogId) },
        respond: { status: 200, json: { id: catalogId, name: "Existing catalog", business: { id: businessId } } },
        times: 1,
      },
      {
        match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${catalogId}/products`) },
        respond: { status: 200, json: { data: [], paging: {} } },
        times: 1,
      },
    )
    const since = await fake.cursor()
    const response = await api.post<Connection>("/v1/products/meta-catalog/select", { catalogId })
    expect(response.status, response.text).toBe(202)
    expect(response.json).toMatchObject({ id: connectionId, catalogId, catalogName: "Existing catalog", businessId })
    expect(["queued", "running"]).toContain(response.json.importStatus)

    const done = await wait(async () => {
      const current = await state(api)
      expect(current.connection?.importStatus).toBe("succeeded")
      expect(current.history).toHaveLength(1)
      expect(current.history[0]?.status).toBe("succeeded")
      return current
    })
    expect(done.history[0]).toMatchObject({ integrationMetaCatalogId: connectionId, catalogId, direction: "import" })
    const calls = await upstreamCalls({ host: GRAPH, since, bearer: accessToken })
    expect(calls.map((call) => call.path.replace(/^\/v[0-9.]+/, ""))).toEqual([`/${catalogId}`, `/${catalogId}/products`])
    expect(calls[0]?.query).toMatchObject({ fields: "id,name,business" })
    expectInvalid(await api.post("/v1/products/meta-catalog/select", { catalogId: "x1" }), "catalogId")
  })
})

describe("products.syncMetaCatalog", () => {
  it("needs a bound catalog; a sync of a store without products finishes without a Meta call", async () => {
    const { tenant, api } = await newTenant("p114-mc-sync")
    const { accessToken } = await connect(tenant)
    expectValidation(
      await api.post("/v1/products/meta-catalog/sync", { scope: "all" }),
      "No catalog is bound yet: send catalogId",
    )
    expectInvalid(await api.post("/v1/products/meta-catalog/sync", { scope: "category" }), "scope")
    expectInvalid(await api.post("/v1/products/meta-catalog/sync", { scope: "selected", selectedProductIds: [] }), "scope")

    const { tenant: bound, api: boundApi } = await newTenant("p114-mc-sync-bound")
    const catalogId = uniqueDigits()
    const second = await connect(bound, { catalogId, catalogName: "Bound", businessId: uniqueDigits() })
    const since = await fake.cursor()
    const response = await boundApi.post<Run>("/v1/products/meta-catalog/sync", { scope: "all" })
    expect(response.status, response.text).toBe(202)
    expect(response.json).toMatchObject({
      integrationMetaCatalogId: second.connectionId,
      catalogId,
      scope: "all",
      direction: "push",
    })
    expect(response.json).not.toHaveProperty("handles")
    expect(response.json).not.toHaveProperty("workspaceId")
    await wait(async () => {
      const current = await state(boundApi)
      expect(current.history[0]?.status).toBe("succeeded")
    })
    expect(await upstreamCalls({ host: GRAPH, since, bearer: second.accessToken })).toHaveLength(0)
    expect(await upstreamCalls({ host: GRAPH, since, bearer: accessToken })).toHaveLength(0)
  })
})

describe("products.disconnectMetaCatalog", () => {
  it("removes the connection (204)", async () => {
    const { tenant, api } = await newTenant("p114-mc-disconnect")
    await connect(tenant, { catalogId: uniqueDigits(), catalogName: "Gone", businessId: uniqueDigits() })
    const response = await api.delete("/v1/products/meta-catalog")
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expect((await state(api)).connection).toBeNull()
    expect((await api.get("/v1/products/meta-catalog/items")).json).toEqual({ connected: false, items: [] })
    expectNotFound(await api.get("/v1/products/meta-catalog/businesses"), "Meta Catalog integration not found")
  })
})
