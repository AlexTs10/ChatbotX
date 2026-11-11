// connections.* with an `api_key` integration provider (DeepSeek): the key is
// checked live against the provider's "list models" endpoint (fake upstream
// `api.deepseek.com/models`) on create and on verify.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import { type Api, CONNECTION_KEYS, expectError, qs, script, tenantApi, upstreamCalls } from "./helpers.ts"

const HOST = "api.deepseek.com"

type Connection = Record<string, unknown> & { id: string; integrationId: string | null; status: string }
type Envelope = { connection: Connection | null; session: unknown; secret: unknown }

/** Answers the next DeepSeek key check with `status`. */
const answerKeyCheck = (status: number) =>
  script({
    match: { host: HOST, method: "GET", path: "/models" },
    respond: status === 200 ? { status, json: { object: "list", data: [] } } : { status, json: { error: "x" } },
    times: 1,
  })

const connectDeepseek = async (api: Api, apiKey = `sk-${uniqueId()}`) => {
  await answerKeyCheck(200)
  const response = await api.post<Envelope>("/v1/connections", { provider: "deepseek", config: { apiKey } })
  expect(response.status).toBe(201)
  return { apiKey, connection: response.json.connection as Connection, response }
}

describe("connections.create (deepseek, api_key)", () => {
  it("validates the key upstream and connects the integration (201)", async () => {
    const { api } = await tenantApi("p105-ds-create")
    const since = await fake.cursor()
    const { apiKey, connection, response } = await connectDeepseek(api)
    expect(Object.keys(connection).sort()).toEqual(CONNECTION_KEYS)
    const normalizer = new Normalizer()
    expect(
      normalizer.normalize(response.json, {
        replace: { [connection.id]: "<connection>", [connection.integrationId as string]: "<integration>" },
      }),
    ).toEqual({
      connection: {
        authExpiresAt: null,
        capabilities: { multiAccount: false, refreshable: false, verifiable: true },
        channel: null,
        connectedAt: "<ts>",
        createdAt: "<ts>",
        disconnectedAt: null,
        displayName: "DeepSeek",
        id: "<connection>",
        inboxId: null,
        integrationId: "<integration>",
        kind: "integration",
        lastError: null,
        provider: "deepseek",
        sourceId: "workspace",
        status: "connected",
        statusReason: null,
        strategy: "api_key",
        updatedAt: "<ts>",
      },
      secret: null,
      session: null,
    })
    const calls = await upstreamCalls({ host: HOST, since, path: /^\/models$/ })
    expect(calls).toHaveLength(1)
    expect(calls[0]?.method).toBe("GET")
    expect(calls[0]?.headers.authorization).toBe(`Bearer ${apiKey}`)
  })

  it("refuses a second connect of a single-account provider (409 connectionAlreadyConnected)", async () => {
    const { api } = await tenantApi("p105-ds-dup")
    await connectDeepseek(api)
    await answerKeyCheck(200)
    expectError(
      await api.post("/v1/connections", { provider: "deepseek", config: { apiKey: `sk-${uniqueId()}` } }),
      409,
      "connectionAlreadyConnected",
      { defined: true },
    )
    const providers = await api.get<{ data: Array<{ provider: string; available: boolean; unavailableReason: string }> }>(
      `/v1/connection-providers${qs({ kind: "integration" })}`,
    )
    expect(providers.json.data.find((p) => p.provider === "deepseek")).toMatchObject({
      available: false,
      unavailableReason: "alreadyConnected",
    })
  })

  it("maps a rejected key (401) to 400 connectionCredentialsRejected and writes nothing", async () => {
    const { api } = await tenantApi("p105-ds-reject")
    await answerKeyCheck(401)
    expectError(
      await api.post("/v1/connections", { provider: "deepseek", config: { apiKey: "sk-bad" } }),
      400,
      "connectionCredentialsRejected",
      { defined: true },
    )
    expect((await api.get<{ data: unknown[] }>("/v1/connections")).json.data).toEqual([])
  })

  it("maps an inconclusive check (upstream 500) to 502 connectionProviderUnavailable", async () => {
    const { api } = await tenantApi("p105-ds-unavail")
    await answerKeyCheck(500)
    expectError(
      await api.post("/v1/connections", { provider: "deepseek", config: { apiKey: "sk-x" } }),
      502,
      "connectionProviderUnavailable",
      { defined: true, message: "The provider is temporarily unavailable. Please try again." },
    )
  })

  it("validates the config before calling the provider (422 validation)", async () => {
    const { api } = await tenantApi("p105-ds-cfg")
    const since = await fake.cursor()
    expectError(await api.post("/v1/connections", { provider: "deepseek", config: {} }), 422, "validation", {
      message: "apiKey is required",
    })
    expectError(await api.post("/v1/connections", { provider: "deepseek" }), 422, "validation")
    expectError(
      await api.post("/v1/connections", { provider: "deepseek", config: { apiKey: 42 } }),
      422,
      "validation",
      { message: "apiKey must be a string" },
    )
    expectError(
      await api.post("/v1/connections", { provider: "deepseek", config: { apiKey: "sk-1", workspaceId: "1" } }),
      422,
      "validation",
      { message: "Unsupported config field(s) for deepseek: workspaceId" },
    )
    expect(await upstreamCalls({ host: HOST, since })).toEqual([])
  })

  it("refuses an integration provider for a channels-only token (403)", async () => {
    const { api } = await tenantApi("p105-ds-chan", { scopes: ["channels"] })
    expectError(
      await api.post("/v1/connections", { provider: "deepseek", config: { apiKey: "sk-x" } }),
      403,
      "FORBIDDEN",
      { message: "Token is not authorized for the 'integrations' scope" },
    )
  })
})

describe("connections.verify / refresh / reconnect / disconnect (deepseek)", () => {
  it("verify: ok keeps connected, 401 marks needs_reauth, an inconclusive check degrades, ok recovers", async () => {
    const { api } = await tenantApi("p105-ds-verify")
    const { apiKey, connection } = await connectDeepseek(api)
    const verify = () => api.post<Connection>(`/v1/connections/${connection.id}/verify`, {})

    const since = await fake.cursor()
    await answerKeyCheck(200)
    const ok = await verify()
    expect(ok.status).toBe(200)
    expect(ok.json).toMatchObject({ status: "connected", statusReason: null })
    const calls = await upstreamCalls({ host: HOST, since, path: /^\/models$/ })
    expect(calls.map((c) => c.headers.authorization)).toEqual([`Bearer ${apiKey}`])

    await answerKeyCheck(500)
    const degraded = await verify()
    expect(degraded.status).toBe(200)
    expect(degraded.json).toMatchObject({ status: "degraded", statusReason: "verify_failed" })

    await answerKeyCheck(200)
    expect((await verify()).json).toMatchObject({ status: "connected", statusReason: null })

    await answerKeyCheck(401)
    const revoked = await verify()
    expect(revoked.status).toBe(200)
    expect(revoked.json).toMatchObject({ status: "needs_reauth", statusReason: "token_revoked" })
    // needs_reauth is not active: verify and refresh are refused.
    expectError(await verify(), 409, "connectionInactive")
    expectError(await api.post(`/v1/connections/${connection.id}/refresh`, {}), 409, "connectionInactive")
  })

  it("refresh is refused (400 connectionNotRefreshable); reconnect has no OAuth (400 connectionNotConfigured)", async () => {
    const { api } = await tenantApi("p105-ds-refresh")
    const { connection } = await connectDeepseek(api)
    expectError(await api.post(`/v1/connections/${connection.id}/refresh`, {}), 400, "connectionNotRefreshable", {
      defined: true,
    })
    expectError(await api.post(`/v1/connections/${connection.id}/reconnect`, {}), 400, "connectionNotConfigured", {
      defined: true,
    })
  })

  it("disconnect needs no upstream call and removes the connection row; a new connect makes a new one", async () => {
    const { api } = await tenantApi("p105-ds-disc")
    const { connection } = await connectDeepseek(api)
    const since = await fake.cursor()
    const disconnected = await api.delete<Connection>(`/v1/connections/${connection.id}`)
    expect(disconnected.status).toBe(200)
    expect(disconnected.json).toMatchObject({
      id: connection.id,
      status: "disconnected",
      statusReason: expect.any(String),
      disconnectedAt: expect.any(String),
    })
    expect(await upstreamCalls({ host: HOST, since })).toEqual([])
    // The binding deletes the parent `Integration` row; the FK cascade deletes
    // the `Connection` row too, so the disconnected connection is gone.
    expectError(await api.get(`/v1/connections/${connection.id}`), 404, "notFound")
    expect((await api.get<{ data: unknown[] }>("/v1/connections")).json.data).toEqual([])
    const providers = await api.get<{ data: Array<{ provider: string; available: boolean }> }>(
      `/v1/connection-providers${qs({ kind: "integration" })}`,
    )
    expect(providers.json.data.find((p) => p.provider === "deepseek")?.available).toBe(true)

    const revived = await connectDeepseek(api)
    expect(revived.connection.id).not.toBe(connection.id)
    expect(revived.connection).toMatchObject({
      status: "connected",
      statusReason: null,
      disconnectedAt: null,
    })
  })
})
