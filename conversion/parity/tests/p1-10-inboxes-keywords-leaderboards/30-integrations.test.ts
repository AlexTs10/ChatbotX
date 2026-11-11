// integrations.list (GET /v1/integrations, paged in memory), integrations.get
// (GET /v1/integrations/{id}) and integrations.tokenErrors (GET
// /v1/integrations/status/token-errors). `Integration` rows and channel rows
// with a failed token refresh come from the P1.10 fixture extension.

import { describe, expect, it } from "vitest"
import { unordered } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  connectInstagram,
  expectInvalid,
  expectNotFound,
  ext,
  ISO,
  newTenant,
  UNKNOWN_ID,
} from "./helpers.ts"

type Integration = { id: string; integrationType: string; createdAt: string; updatedAt: string }
const KEYS = ["createdAt", "id", "integrationType", "updatedAt"]

describe("integrations.list", () => {
  it("an empty workspace still reports one page", async () => {
    const { api } = await newTenant("p110-int-empty")
    const response = await api.get("/v1/integrations")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ data: [], pageCount: 1 })
  })

  it("lists the workspace's integrations; a metaCatalog row without a catalog is hidden", async () => {
    const { tenant, api } = await newTenant("p110-int-list")
    const fx = await ext()
    const sheets = await fx.createIntegration({ workspaceId: tenant.workspaceId, integrationType: "googleSheets" })
    const mailchimp = await fx.createIntegration({ workspaceId: tenant.workspaceId, integrationType: "mailchimp" })
    await fx.createIntegration({ workspaceId: tenant.workspaceId, integrationType: "metaCatalog" })
    const { tenant: other } = await newTenant("p110-int-list-o")
    await fx.createIntegration({ workspaceId: other.workspaceId, integrationType: "googleSheets" })

    const response = await api.get<{ data: Integration[]; pageCount: number }>("/v1/integrations")
    expect(response.status, response.text).toBe(200)
    expect(response.json.pageCount).toBe(1)
    for (const item of response.json.data) {
      expect(Object.keys(item).sort()).toEqual(KEYS)
      expect(item.createdAt).toMatch(ISO)
      expect(item.updatedAt).toMatch(ISO)
    }
    expect(unordered(response.json.data.map(({ id, integrationType }) => ({ id, integrationType })), (item) => item.id)).toEqual(
      unordered(
        [
          { id: sheets.integrationId, integrationType: "googleSheets" },
          { id: mailchimp.integrationId, integrationType: "mailchimp" },
        ],
        (item) => item.id,
      ),
    )
  })

  it("pages with page/perPage (default 50 per page, max 50)", async () => {
    const { tenant, api } = await newTenant("p110-int-page")
    const fx = await ext()
    const ids: string[] = []
    for (const type of ["googleSheets", "mailchimp", "klaviyo"]) {
      ids.push((await fx.createIntegration({ workspaceId: tenant.workspaceId, integrationType: type })).integrationId)
    }
    const seen: string[] = []
    for (const page of [1, 2]) {
      const response = await api.get<{ data: Integration[]; pageCount: number }>("/v1/integrations", {
        query: { page, perPage: 2 },
      })
      expect(response.status, response.text).toBe(200)
      expect(response.json.pageCount).toBe(2)
      expect(response.json.data).toHaveLength(page === 1 ? 2 : 1)
      seen.push(...response.json.data.map((item) => item.id))
    }
    expect(seen.sort()).toEqual(ids.sort())
    const beyond = await api.get("/v1/integrations", { query: { page: 3, perPage: 2 } })
    expect(beyond.json).toEqual({ data: [], pageCount: 2 })

    expectInvalid(await api.get("/v1/integrations", { query: { perPage: 51 } }), "perPage")
    expectInvalid(await api.get("/v1/integrations", { query: { perPage: 0 } }), "perPage")
    expectInvalid(await api.get("/v1/integrations", { query: { page: 0 } }), "page")
  })
})

describe("integrations.get", () => {
  it("returns one integration (also a hidden metaCatalog row); 404 for unknown or foreign ids", async () => {
    const { tenant, api } = await newTenant("p110-int-get")
    const fx = await ext()
    const { integrationId } = await fx.createIntegration({ workspaceId: tenant.workspaceId, integrationType: "googleSheets" })
    const response = await api.get<Integration>(`/v1/integrations/${integrationId}`)
    expect(response.status, response.text).toBe(200)
    expect(Object.keys(response.json).sort()).toEqual(KEYS)
    expect(response.json).toMatchObject({ id: integrationId, integrationType: "googleSheets" })

    const catalog = await fx.createIntegration({ workspaceId: tenant.workspaceId, integrationType: "metaCatalog" })
    const hidden = await api.get<Integration>(`/v1/integrations/${catalog.integrationId}`)
    expect(hidden.status, hidden.text).toBe(200)
    expect(hidden.json.integrationType).toBe("metaCatalog")

    expectNotFound(await api.get(`/v1/integrations/${UNKNOWN_ID}`), "Integration not found")
    const { api: otherApi } = await newTenant("p110-int-get-o")
    expectNotFound(await otherApi.get(`/v1/integrations/${integrationId}`), "Integration not found")
  })
})

describe("integrations.tokenErrors", () => {
  it("is empty when no channel token refresh failed", async () => {
    const { tenant, api } = await newTenant("p110-int-tokens-none")
    await connectInstagram(tenant, "instagram")
    const response = await api.get("/v1/integrations/status/token-errors")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ data: [] })
  })

  it("lists channel integrations whose refresh failed, Zalo before Instagram; instagramFacebook for Facebook Login", async () => {
    const { tenant, api } = await newTenant("p110-int-tokens")
    const fx = await ext()
    const zaloName = `Zalo ${uniqueId()}`
    const zalo = await fx.createZaloIntegration({
      workspaceId: tenant.workspaceId,
      name: zaloName,
      oaId: `oa-${uniqueId()}`,
      tokenRefreshError: "Zalo refresh token expired",
    })
    await fx.createZaloIntegration({ workspaceId: tenant.workspaceId, name: "Fine", oaId: `oa-${uniqueId()}` })
    const native = await connectInstagram(tenant, "instagram", { tokenRefreshError: "Session expired" })
    const viaFacebook = await connectInstagram(tenant, "facebook", { tokenRefreshError: "Password changed" })
    await connectInstagram(tenant, "facebook")
    const { tenant: other } = await newTenant("p110-int-tokens-o")
    await connectInstagram(other, "instagram", { tokenRefreshError: "Other workspace" })

    const response = await api.get<{ data: Array<{ id: string; channel: string; name: string; error: string }> }>(
      "/v1/integrations/status/token-errors",
    )
    expect(response.status, response.text).toBe(200)
    const [first, ...rest] = response.json.data
    expect(first).toEqual({ id: zalo.integrationId, channel: "zalo", name: zaloName, error: "Zalo refresh token expired" })
    expect(unordered(rest, (item) => item.id)).toEqual(
      unordered(
        [
          { id: native.integrationId, channel: "instagram", name: native.name, error: "Session expired" },
          { id: viaFacebook.integrationId, channel: "instagramFacebook", name: viaFacebook.name, error: "Password changed" },
        ],
        (item) => item.id,
      ),
    )
  })
})
