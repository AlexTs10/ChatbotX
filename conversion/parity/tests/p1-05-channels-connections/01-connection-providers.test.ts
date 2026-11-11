// connectionProviders.list — GET /v1/connection-providers.
// The connect catalog: one entry per registered provider (facebookAds and the
// providers without an adapter are left out), with `available` /
// `unavailableReason` resolved against the workspace.
//
// Original behavior locked in here: every request whose catalog includes the
// channel-kind providers (no `kind`, or `kind=channel`) fails with 422
// `invalidRequestData` (`defined:false`, `data` = a bare zod issue array with
// `path: []`): the `chatbotx` provider is a channel-kind adapter whose name is
// not in `channelTypes`, so `toChannelType` throws a ZodError. Only
// `kind=integration` (or an integrations-only token) returns the catalog.

import { describe, expect, it } from "vitest"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import { type Api, expectError, expectInvalid, qs, script, tenantApi } from "./helpers.ts"

type Provider = {
  provider: string
  kind: "channel" | "integration"
  channel: string | null
  strategy: string
  multiAccount: boolean
  multiInstance: boolean
  configFields: Array<Record<string, unknown>>
  available: boolean
  unavailableReason: string | null
}
type ProviderList = { data: Provider[]; pageCount: number }

const PROVIDER_KEYS = [
  "available",
  "channel",
  "configFields",
  "kind",
  "multiAccount",
  "multiInstance",
  "provider",
  "strategy",
  "unavailableReason",
]

const CHANNEL_TYPES = [
  "omnichannel",
  "webchat",
  "messenger",
  "whatsapp",
  "zalo",
  "smtp",
  "telegram",
  "instagram",
  "threads",
  "tiktok",
  "api",
]

/** The 422 the original answers when the catalog includes the channel-kind providers. */
const expectChannelCatalogFailure = (response: Awaited<ReturnType<Api["get"]>>) => {
  expectError(response, 422, "invalidRequestData", { defined: false })
  const data = (response.json as { data: Array<Record<string, unknown>> }).data
  expect(data).toEqual([
    expect.objectContaining({ code: "invalid_value", path: [], values: CHANNEL_TYPES }),
  ])
}

const integrations = async (api: Api) => {
  const response = await api.get<ProviderList>(`/v1/connection-providers${qs({ kind: "integration" })}`)
  expect(response.status).toBe(200)
  return response.json
}

describe("connectionProviders.list", () => {
  it("fails with 422 when the catalog includes the channel providers (no kind, kind=channel)", async () => {
    const { api } = await tenantApi("p105-prov-all")
    expectChannelCatalogFailure(await api.get("/v1/connection-providers"))
    expectChannelCatalogFailure(await api.get(`/v1/connection-providers${qs({ kind: "channel" })}`))
    // A channels-only token gets kind=channel forced: same failure.
    const { api: channelsOnly } = await tenantApi("p105-prov-chan", { scopes: ["channels"] })
    expectChannelCatalogFailure(await channelsOnly.get("/v1/connection-providers"))
  })

  it("returns the integration catalog with one page and the documented entry shape", async () => {
    const { api } = await tenantApi("p105-prov-int")
    const list = await integrations(api)
    expect(list.pageCount).toBe(1)
    for (const entry of list.data) {
      expect(Object.keys(entry).sort()).toEqual(PROVIDER_KEYS)
      expect(entry).toMatchObject({ kind: "integration", channel: null })
    }
    const names = list.data.map((p) => p.provider)
    expect(new Set(names).size).toBe(names.length)
    expect(names).not.toContain("facebookAds")
    expect(names).not.toContain("metaCatalog")
    expect(names).not.toContain("outlookCalendar")
    expect(names).toEqual(expect.arrayContaining(["deepseek", "openai", "claude", "googleSheets"]))
    const byName = new Map(list.data.map((p) => [p.provider, p]))
    expect(byName.get("deepseek")).toMatchObject({
      strategy: "api_key",
      multiAccount: false,
      multiInstance: false,
      available: true,
      unavailableReason: null,
      configFields: [expect.objectContaining({ name: "apiKey", type: "secret", required: true })],
    })
    // OAuth provider whose owner has no platform credential.
    expect(byName.get("googleSheets")).toMatchObject({
      strategy: "oauth_redirect",
      available: false,
      unavailableReason: "credentialMissing",
    })
    expect(new Normalizer().normalize(list.data)).toMatchSnapshot()
  })

  it("an integrations-only token gets the integration catalog without a kind", async () => {
    const { api } = await tenantApi("p105-prov-intonly", { scopes: ["integrations"] })
    const response = await api.get<ProviderList>("/v1/connection-providers")
    expect(response.status).toBe(200)
    expect(response.json.data.length).toBeGreaterThan(0)
    expect(response.json.data.every((p) => p.kind === "integration")).toBe(true)
  })

  it("refuses a kind outside the token's scopes (403) and an unknown kind (422)", async () => {
    const { api: channelsOnly } = await tenantApi("p105-prov-chan403", { scopes: ["channels"] })
    const { api: integrationsOnly } = await tenantApi("p105-prov-int403", { scopes: ["integrations"] })
    expectError(
      await channelsOnly.get(`/v1/connection-providers${qs({ kind: "integration" })}`),
      403,
      "FORBIDDEN",
      { message: "Token is not authorized for the 'integrations' scope" },
    )
    expectError(
      await integrationsOnly.get(`/v1/connection-providers${qs({ kind: "channel" })}`),
      403,
      "FORBIDDEN",
      { message: "Token is not authorized for the 'channels' scope" },
    )
    const { api } = await tenantApi("p105-prov-badkind")
    expectInvalid(await api.get(`/v1/connection-providers${qs({ kind: "robot" })}`), "kind")
  })

  it("marks a single-account provider alreadyConnected while it has a live connection", async () => {
    const { api } = await tenantApi("p105-prov-already")
    await script({
      match: { host: "api.deepseek.com", method: "GET", path: "/models" },
      respond: { status: 200, json: { object: "list", data: [] } },
      times: 1,
    })
    const created = await api.post<{ connection: { id: string } }>("/v1/connections", {
      provider: "deepseek",
      config: { apiKey: `sk-${uniqueId()}` },
    })
    expect(created.status).toBe(201)
    const find = async () => (await integrations(api)).data.find((p) => p.provider === "deepseek")
    expect(await find()).toMatchObject({ available: false, unavailableReason: "alreadyConnected" })
    expect((await api.delete(`/v1/connections/${created.json.connection.id}`)).status).toBe(200)
    // A disconnected row no longer blocks a new connect.
    expect(await find()).toMatchObject({ available: true, unavailableReason: null })
  })
})
