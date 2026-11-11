// connections.* with the self-serve `api` channel (and `webchat`), which needs
// no third party: create / get / list / update / verify / refresh /
// reconnect / disconnect, validation, scope rules and workspace isolation.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { fixtures } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueHost } from "../../src/unique.ts"
import {
  type Api,
  anyIssuePaths,
  CONNECTION_KEYS,
  expectError,
  expectInvalid,
  qs,
  tenantApi,
  uniqueName,
} from "./helpers.ts"

type Connection = Record<string, unknown> & {
  id: string
  inboxId: string | null
  sourceId: string
  status: string
  displayName: string
  updatedAt: string
}
type Envelope = { connection: Connection | null; session: unknown; secret: { kind: string; token: string } | null }
type ConnectionList = { data: Connection[]; pageCount: number }

const apiName = (label: string) => uniqueName(label).slice(0, 40)

const createApiChannel = async (api: Api, name = apiName("api"), callbackUrl?: string | null) => {
  const response = await api.post<Envelope>("/v1/connections", {
    provider: "api",
    config: { name, ...(callbackUrl !== undefined ? { callbackUrl } : {}) },
  })
  expect(response.status).toBe(201)
  return { response, connection: response.json.connection as Connection, secret: response.json.secret }
}

/** Normalized `api` connection with its own ids replaced by stable labels. */
const expectedApiConnection = (overrides: Record<string, unknown> = {}) => ({
  authExpiresAt: null,
  capabilities: { multiAccount: false, refreshable: false, verifiable: true },
  channel: "api",
  connectedAt: "<ts>",
  createdAt: "<ts>",
  disconnectedAt: null,
  displayName: expect.any(String),
  id: "<connection>",
  inboxId: "<inbox>",
  integrationId: null,
  kind: "channel",
  lastError: null,
  provider: "api",
  sourceId: "<inbox>",
  status: "connected",
  statusReason: null,
  strategy: "self_serve",
  updatedAt: "<ts>",
  ...overrides,
})

const labels = (connection: Connection) => ({
  [connection.id]: "<connection>",
  [connection.inboxId as string]: "<inbox>",
})

describe("connections.create (api, self-serve)", () => {
  it("connects an API channel at once and returns its bearer token once (201)", async () => {
    const { api } = await tenantApi("p105-api-create")
    const name = apiName("api-create")
    const { response, connection } = await createApiChannel(api, name)
    expect(Object.keys(connection).sort()).toEqual(CONNECTION_KEYS)
    expect(connection.inboxId).toBe(connection.sourceId)
    const normalizer = new Normalizer()
    expect(normalizer.normalize(response.json, { replace: labels(connection) })).toEqual({
      connection: expectedApiConnection({ displayName: name }),
      secret: { kind: "api_channel_token", token: "<cbx_api>" },
      session: null,
    })
    // The token authenticates the new channel.
    const me = await apiClient(response.json.secret?.token as string).get<Record<string, string>>("/v1/channels/api/me")
    expect(me.status).toBe(200)
    expect(me.json).toMatchObject({ inboxId: connection.inboxId, inboxName: name })
  })

  it("accepts a public callbackUrl", async () => {
    const { api } = await tenantApi("p105-api-cb")
    const { connection } = await createApiChannel(api, apiName("cb"), `https://${uniqueHost("cb")}/hook`)
    expect(connection.status).toBe("connected")
  })

  it("refuses a callbackUrl that resolves to a private address (422 invalidRequestData)", async () => {
    const { api } = await tenantApi("p105-api-ssrf")
    const host = uniqueHost("private-cb")
    await fake.dns({ name: host, a: ["10.20.30.40"] })
    const response = await api.post("/v1/connections", {
      provider: "api",
      config: { name: apiName("ssrf"), callbackUrl: `https://${host}/hook` },
    })
    expectError(response, 422, "invalidRequestData")
    const list = await api.get<ConnectionList>("/v1/connections")
    expect(list.json.data).toEqual([])
  })

  it("validates the provider config (name 1-40, callbackUrl URL)", async () => {
    const { api } = await tenantApi("p105-api-cfg")
    const missing = await api.post("/v1/connections", { provider: "api", config: {} })
    expectError(missing, 422, "invalidRequestData")
    expect(anyIssuePaths(missing)).toContain("name")
    const tooLong = await api.post("/v1/connections", { provider: "api", config: { name: "x".repeat(41) } })
    expectError(tooLong, 422, "invalidRequestData")
    expect(anyIssuePaths(tooLong)).toContain("name")
    const badUrl = await api.post("/v1/connections", {
      provider: "api",
      config: { name: "API", callbackUrl: "not a url" },
    })
    expectError(badUrl, 422, "invalidRequestData")
    expect(anyIssuePaths(badUrl)).toContain("callbackUrl")
  })

  it("validates the request body (provider enum, redirectUrl URL)", async () => {
    const { api } = await tenantApi("p105-api-body")
    expectInvalid(await api.post("/v1/connections", {}), "provider")
    expectInvalid(await api.post("/v1/connections", { provider: "fax" }), "provider")
    expectInvalid(
      await api.post("/v1/connections", { provider: "api", config: { name: "A" }, redirectUrl: "nope" }),
      "redirectUrl",
    )
  })

  it("refuses googleAds on the generic API (403 connectionProviderDedicatedOnly)", async () => {
    const { api } = await tenantApi("p105-api-gads")
    expectError(await api.post("/v1/connections", { provider: "googleAds" }), 403, "connectionProviderDedicatedOnly", {
      defined: true,
    })
  })

  it("refuses facebookAds (400 connectionNotOAuth)", async () => {
    const { api } = await tenantApi("p105-api-fbads")
    expectError(await api.post("/v1/connections", { provider: "facebookAds" }), 400, "connectionNotOAuth", {
      defined: true,
    })
  })

  it("refuses a provider with no adapter (400 connectionNotConfigured)", async () => {
    const { api } = await tenantApi("p105-api-nocfg")
    expectError(await api.post("/v1/connections", { provider: "metaCatalog" }), 400, "connectionNotConfigured", {
      defined: true,
    })
  })

  it("checks the provider's kind against the token scope", async () => {
    const { api: integrationsOnly } = await tenantApi("p105-api-intonly", { scopes: ["integrations"] })
    expectError(
      await integrationsOnly.post("/v1/connections", { provider: "api", config: { name: "A" } }),
      403,
      "FORBIDDEN",
      { message: "Token is not authorized for the 'channels' scope" },
    )
    const { api: channelsOnly } = await tenantApi("p105-api-chanonly", { scopes: ["channels"] })
    const { connection } = await createApiChannel(channelsOnly)
    expect(connection.kind).toBe("channel")
  })

  it("connects a webchat channel (self-serve, no secret)", async () => {
    const { api } = await tenantApi("p105-webchat")
    const name = apiName("webchat")
    const response = await api.post<Envelope>("/v1/connections", { provider: "webchat", config: { name } })
    expect(response.status).toBe(201)
    expect(response.json.session).toBeNull()
    expect(response.json.secret).toBeNull()
    expect(response.json.connection).toMatchObject({
      kind: "channel",
      provider: "webchat",
      channel: "webchat",
      status: "connected",
      strategy: "self_serve",
      displayName: name,
    })
    expectError(await api.post("/v1/connections", { provider: "webchat", config: {} }), 422, "invalidRequestData")
  })
})

describe("connections.get / list", () => {
  it("gets a connection by id with the same shape as create", async () => {
    const { api } = await tenantApi("p105-api-get")
    const { connection } = await createApiChannel(api)
    const response = await api.get<Connection>(`/v1/connections/${connection.id}`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual(connection)
  })

  it("404s an unknown id and another workspace's connection", async () => {
    const { api } = await tenantApi("p105-api-get404")
    const { api: other } = await tenantApi("p105-api-get404-other")
    const { connection } = await createApiChannel(other)
    expectError(await api.get("/v1/connections/1"), 404, "notFound", { defined: true, message: "Connection not found" })
    expectError(await api.get(`/v1/connections/${connection.id}`), 404, "notFound", { defined: true })
    expectError(await api.put(`/v1/connections/${connection.id}`, { displayName: "x" }), 404, "notFound")
    expectError(await api.delete(`/v1/connections/${connection.id}`), 404, "notFound")
    expectError(await api.post(`/v1/connections/${connection.id}/verify`, {}), 404, "notFound")
    expectError(await api.post(`/v1/connections/${connection.id}/refresh`, {}), 404, "notFound")
    expectError(await api.post(`/v1/connections/${connection.id}/reconnect`, {}), 404, "notFound")
    // The other workspace still sees it.
    expect((await other.get(`/v1/connections/${connection.id}`)).status).toBe(200)
  })

  it("lists the workspace's connections ordered by kind, provider, display name", async () => {
    const { api } = await tenantApi("p105-api-list")
    const b = await createApiChannel(api, "B api channel")
    const a = await createApiChannel(api, "A api channel")
    const c = await createApiChannel(api, "C api channel")
    const response = await api.get<ConnectionList>("/v1/connections")
    expect(response.status).toBe(200)
    expect(response.json.pageCount).toBe(1)
    expect(response.json.data.map((item) => item.id)).toEqual([a.connection.id, b.connection.id, c.connection.id])
    expect(response.json.data[0]).toEqual(a.connection)
  })

  it("pages with page/perPage and validates them", async () => {
    const { api } = await tenantApi("p105-api-page")
    const first = await createApiChannel(api, "P1 api")
    const second = await createApiChannel(api, "P2 api")
    await createApiChannel(api, "P3 api")
    const page1 = await api.get<ConnectionList>(`/v1/connections${qs({ perPage: 2 })}`)
    expect(page1.json).toMatchObject({ pageCount: 2 })
    expect(page1.json.data.map((item) => item.id)).toEqual([first.connection.id, second.connection.id])
    const page2 = await api.get<ConnectionList>(`/v1/connections${qs({ perPage: 2, page: 2 })}`)
    expect(page2.json.data).toHaveLength(1)
    const empty = await api.get<ConnectionList>(`/v1/connections${qs({ perPage: 2, page: 3 })}`)
    expect(empty.json).toEqual({ data: [], pageCount: 2 })
    expectInvalid(await api.get(`/v1/connections${qs({ perPage: 51 })}`), "perPage")
    expectInvalid(await api.get(`/v1/connections${qs({ perPage: 0 })}`), "perPage")
    expectInvalid(await api.get(`/v1/connections${qs({ page: 0 })}`), "page")
  })

  it("filters by kind, provider, channel and status", async () => {
    const { api } = await tenantApi("p105-api-filter")
    const kept = await createApiChannel(api, "Kept api")
    const gone = await createApiChannel(api, "Gone api")
    expect((await api.delete(`/v1/connections/${gone.connection.id}`)).status).toBe(200)
    const ids = async (query: Record<string, string>) =>
      (await api.get<ConnectionList>(`/v1/connections${qs(query)}`)).json.data.map((item) => item.id)
    expect(await ids({ kind: "channel" })).toEqual([gone.connection.id, kept.connection.id])
    expect(await ids({ kind: "integration" })).toEqual([])
    expect(await ids({ provider: "api" })).toHaveLength(2)
    expect(await ids({ provider: "telegram" })).toEqual([])
    expect(await ids({ channel: "api" })).toHaveLength(2)
    expect(await ids({ status: "connected" })).toEqual([kept.connection.id])
    expect(await ids({ status: "disconnected" })).toEqual([gone.connection.id])
    expectInvalid(await api.get(`/v1/connections${qs({ status: "asleep" })}`), "status")
    expectInvalid(await api.get(`/v1/connections${qs({ provider: "fax" })}`), "provider")
    expectInvalid(await api.get(`/v1/connections${qs({ channel: "fax" })}`), "channel")
  })

  it("restricts a single-kind token to its kind", async () => {
    const { tenant, api } = await tenantApi("p105-api-kindtoken")
    await createApiChannel(api)
    const { token } = await (await fixtures()).createWorkspaceToken({
      workspaceId: tenant.workspaceId,
      name: "integrations only",
      permission: "full",
      scopes: ["integrations"],
    })
    const integrationsOnly = apiClient(token)
    const list = await integrationsOnly.get<ConnectionList>("/v1/connections")
    expect(list.status).toBe(200)
    expect(list.json.data).toEqual([])
    expectError(await integrationsOnly.get(`/v1/connections${qs({ kind: "channel" })}`), 403, "FORBIDDEN", {
      message: "Token is not authorized for the 'channels' scope",
    })
    const all = await api.get<ConnectionList>("/v1/connections")
    const [connection] = all.json.data
    expectError(await integrationsOnly.get(`/v1/connections/${connection?.id}`), 403, "FORBIDDEN", {
      message: "Token is not authorized for the 'channels' scope",
    })
    expectError(
      await integrationsOnly.put(`/v1/connections/${connection?.id}`, { displayName: "Renamed" }),
      403,
      "FORBIDDEN",
    )
    // Not renamed.
    expect((await api.get<Connection>(`/v1/connections/${connection?.id}`)).json.displayName).toBe(
      connection?.displayName,
    )
  })
})

describe("connections.update", () => {
  it("renames the connection (display name only, trimmed)", async () => {
    const { api } = await tenantApi("p105-api-rename")
    const { connection } = await createApiChannel(api)
    const response = await api.put<Connection>(`/v1/connections/${connection.id}`, { displayName: "  Renamed API  " })
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ ...connection, displayName: "Renamed API", updatedAt: expect.any(String) })
    expect(Date.parse(response.json.updatedAt)).toBeGreaterThanOrEqual(Date.parse(connection.updatedAt))
    expect((await api.get<Connection>(`/v1/connections/${connection.id}`)).json.displayName).toBe("Renamed API")
  })

  it("validates displayName (1-200 characters after trim)", async () => {
    const { api } = await tenantApi("p105-api-rename-bad")
    const { connection } = await createApiChannel(api)
    expectInvalid(await api.put(`/v1/connections/${connection.id}`, {}), "displayName")
    expectInvalid(await api.put(`/v1/connections/${connection.id}`, { displayName: "   " }), "displayName")
    expectInvalid(await api.put(`/v1/connections/${connection.id}`, { displayName: "x".repeat(201) }), "displayName")
    expect((await api.put(`/v1/connections/${connection.id}`, { displayName: "y".repeat(200) })).status).toBe(200)
  })
})

describe("connections.verify / refresh / reconnect (api)", () => {
  it("verify runs the provider health check and keeps the connection connected", async () => {
    const { api } = await tenantApi("p105-api-verify")
    const { connection } = await createApiChannel(api)
    const response = await api.post<Connection>(`/v1/connections/${connection.id}/verify`, {})
    expect(response.status).toBe(200)
    expect(response.json).toMatchObject({ id: connection.id, status: "connected", statusReason: null, lastError: null })
  })

  it("refresh is refused for a provider without token refresh (400 connectionNotRefreshable)", async () => {
    const { api } = await tenantApi("p105-api-refresh")
    const { connection } = await createApiChannel(api)
    expectError(await api.post(`/v1/connections/${connection.id}/refresh`, {}), 400, "connectionNotRefreshable", {
      defined: true,
    })
  })

  it("reconnect is refused for a non-OAuth provider (400 connectionNotConfigured)", async () => {
    const { api } = await tenantApi("p105-api-reconnect")
    const { connection } = await createApiChannel(api)
    expectError(await api.post(`/v1/connections/${connection.id}/reconnect`, {}), 400, "connectionNotConfigured", {
      defined: true,
    })
    expectInvalid(await api.post(`/v1/connections/${connection.id}/reconnect`, { redirectUrl: "nope" }), "redirectUrl")
  })
})

describe("connections.disconnect (api)", () => {
  it("marks the connection disconnected and keeps the row; the channel token still authenticates", async () => {
    const { api } = await tenantApi("p105-api-disc")
    const { connection, secret } = await createApiChannel(api)
    const response = await api.delete<Connection>(`/v1/connections/${connection.id}`)
    expect(response.status).toBe(200)
    const normalizer = new Normalizer()
    expect(normalizer.normalize(response.json, { replace: labels(connection) })).toEqual(
      expectedApiConnection({
        displayName: connection.displayName,
        status: "disconnected",
        statusReason: expect.any(String),
        disconnectedAt: "<ts>",
      }),
    )
    expect((await api.get<Connection>(`/v1/connections/${connection.id}`)).json.status).toBe("disconnected")
    // The api binding keeps its IntegrationApi row on disconnect (`onDisconnect: "keep_row"`),
    // so the channel token still authenticates.
    const me = await apiClient(secret?.token as string).get("/v1/channels/api/me")
    expect(me.status).toBe(200)
    expect(me.json).toMatchObject({ inboxId: connection.inboxId })
  })

  it("refuses verify and refresh on a disconnected connection (409 connectionInactive)", async () => {
    const { api } = await tenantApi("p105-api-inactive")
    const { connection } = await createApiChannel(api)
    expect((await api.delete(`/v1/connections/${connection.id}`)).status).toBe(200)
    expectError(await api.post(`/v1/connections/${connection.id}/verify`, {}), 409, "connectionInactive", {
      defined: true,
    })
    expectError(await api.post(`/v1/connections/${connection.id}/refresh`, {}), 409, "connectionInactive", {
      defined: true,
    })
    // Rename still works on a disconnected row.
    expect((await api.put(`/v1/connections/${connection.id}`, { displayName: "Old API" })).status).toBe(200)
  })
})

describe("connections.create with Idempotency-Key", () => {
  it("replays the first result without the one-time secret; a different body with the same key is refused", async () => {
    const { api } = await tenantApi("p105-api-idem")
    const key = `idem-${uniqueName("k")}`
    const body = { provider: "api", config: { name: "Idempotent API" } }
    const first = await api.post<Envelope>("/v1/connections", body, { headers: { "idempotency-key": key } })
    expect(first.status).toBe(201)
    expect(first.json.secret?.token).toMatch(/^cbx_api_/)
    const replay = await api.post<Envelope>("/v1/connections", body, { headers: { "idempotency-key": key } })
    expect(replay.status).toBe(201)
    expect(replay.headers.get("idempotent-replayed")).toBe("true")
    expect(replay.json).toEqual({ ...first.json, secret: null })
    expect((await api.get<ConnectionList>("/v1/connections")).json.data).toHaveLength(1)

    const reused = await api.post("/v1/connections", { provider: "api", config: { name: "Other API" } }, {
      headers: { "idempotency-key": key },
    })
    expectError(reused, 422, "idempotencyKeyReused", { defined: true })
    const invalid = await api.post("/v1/connections", body, { headers: { "idempotency-key": "k".repeat(256) } })
    expectError(invalid, 422, "idempotencyKeyInvalid", { defined: true })
  })
})
