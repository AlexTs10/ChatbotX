// OAuth connect sessions with Messenger: connections.create /
// connections.reconnect start a session, connectSessions.get / cancel /
// connectTargets drive it. The OAuth callback
// (`GET /integrations/messenger/callback?code&state`) needs no session cookie,
// so the whole round trip runs over HTTP; Graph calls go to the fake upstream.
//
// The owner's Messenger platform credential is a fixture row (no public HTTP
// path), written before the first credential lookup of the tenant.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { fixtures } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { normalizeUrl } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  type Api,
  appClient,
  CONNECTION_KEYS,
  ext,
  expectError,
  expectInvalid,
  graphPath,
  SESSION_KEYS,
  script,
  tenantApi,
  uniqueDigits,
  upstreamCalls,
} from "./helpers.ts"

const GRAPH = "graph.facebook.com"
const MESSENGER_SCOPES = [
  "email",
  "public_profile",
  "pages_manage_ads",
  "pages_manage_metadata",
  "pages_read_engagement",
  "pages_read_user_content",
  "pages_manage_posts",
  "pages_manage_engagement",
  "pages_messaging",
  "pages_show_list",
  "business_management",
  "pages_utility_messaging",
]
const ADMIN_TASKS = ["ADVERTISE", "ANALYZE", "CREATE_CONTENT", "MANAGE", "MODERATE"]

type Session = {
  id: string
  provider: string
  purpose: string
  status: string
  step: string
  nextAction: { type: string; url: string } | null
  targets: Array<Record<string, unknown>>
  connectionIds: string[]
  errorCode: string | null
  expiresAt: string
}
type Connection = Record<string, unknown> & { id: string; sourceId: string; status: string; inboxId: string | null }
type Envelope = { connection: Connection | null; session: Session | null; secret: unknown }
type TargetsResult = { session: Session; connections: Connection[]; outcomes: Array<Record<string, unknown>> }

const withCredential = async (label: string, options: Parameters<typeof tenantApi>[1] = {}) => {
  const { tenant, api } = await tenantApi(label, options)
  const clientId = `app${uniqueDigits()}`
  await (await ext()).upsertMessengerCredential({
    userId: tenant.userId,
    clientId,
    clientSecret: `secret-${uniqueId()}`,
    version: "v23.0",
    verifyToken: "parity-verify",
  })
  return { tenant, api, clientId }
}

const startMessenger = async (api: Api, body: Record<string, unknown> = {}) => {
  const response = await api.post<Envelope>("/v1/connections", { provider: "messenger", ...body })
  expect(response.status).toBe(201)
  return { response, session: response.json.session as Session }
}

const stateOf = (session: Session): string =>
  new URL(session.nextAction?.url as string).searchParams.get("state") as string

const callback = (query: Record<string, string>, provider = "messenger") =>
  appClient().get(`/integrations/${provider}/callback`, { query, redirect: "manual" })

const expectRedirect = (response: { status: number; headers: Headers }, pathAndQuery: string) => {
  expect([302, 303, 307, 308]).toContain(response.status)
  const location = response.headers.get("location") ?? ""
  expect(new URL(location, "http://app").pathname + new URL(location, "http://app").search).toBe(pathAndQuery)
}

type Page = { id: string; name: string; token: string }
const newPage = (label: string): Page => {
  const id = uniqueDigits()
  return { id, name: `${label} ${id}`, token: `page-token-${id}` }
}

/** Scripts the code exchange (short + long-lived token) and the page list. */
const scriptGraphAuthorization = async (pages: Page[], options: { exchangeStatus?: number } = {}) => {
  const exchangeStatus = options.exchangeStatus ?? 200
  await script({
    match: { host: GRAPH, method: "GET", pathRegex: graphPath("oauth/access_token") },
    respond:
      exchangeStatus === 200
        ? { status: 200, json: { access_token: `user-token-${uniqueId()}`, token_type: "bearer", expires_in: 5_184_000 } }
        : {
            status: exchangeStatus,
            json: { error: { message: "Invalid verification code format.", type: "OAuthException", code: 100 } },
          },
    times: exchangeStatus === 200 ? 2 : 1,
  })
  await script({
    match: { host: GRAPH, method: "GET", pathRegex: graphPath("me/accounts") },
    respond: {
      status: 200,
      json: {
        data: pages.map((page) => ({
          id: page.id,
          name: page.name,
          access_token: page.token,
          tasks: ADMIN_TASKS,
          category: "Software",
        })),
        paging: { cursors: { before: "b", after: "a" } },
      },
    },
    times: 1,
  })
}

const scriptSubscribe = (status = 200) =>
  script({
    match: { host: GRAPH, method: "POST", pathRegex: graphPath("me/subscribed_apps") },
    respond: status === 200 ? { status, json: { success: true } } : { status, json: { error: { message: "boom", code: 1 } } },
    // The Graph client retries a failed call, so a failure answers every attempt.
    times: status === 200 ? 2 : 20,
  })

describe("connections.create (messenger, OAuth)", () => {
  it("refuses an OAuth provider whose owner has no platform credential (400 connectionNotConfigured)", async () => {
    const { api } = await tenantApi("p105-oauth-nocred")
    expectError(await api.post("/v1/connections", { provider: "messenger" }), 400, "connectionNotConfigured", {
      defined: true,
    })
  })

  it("starts a pending session whose next action opens the Facebook dialog", async () => {
    const { api, clientId } = await withCredential("p105-oauth-start")
    const before = Date.now()
    const { response, session } = await startMessenger(api)
    expect(response.json.connection).toBeNull()
    expect(response.json.secret).toBeNull()
    expect(Object.keys(session).sort()).toEqual(SESSION_KEYS)
    expect(session).toMatchObject({
      provider: "messenger",
      purpose: "connect",
      status: "pending",
      step: "authorize",
      targets: [],
      connectionIds: [],
      errorCode: null,
      nextAction: { type: "open_url", url: expect.any(String) },
    })
    const ttl = Date.parse(session.expiresAt) - before
    expect(ttl).toBeGreaterThan(9 * 60_000)
    expect(ttl).toBeLessThanOrEqual(10 * 60_000 + 5_000)

    const url = new URL(session.nextAction?.url as string)
    expect(`${url.origin}${url.pathname}`).toBe("https://www.facebook.com/v23.0/dialog/oauth")
    const params = Object.fromEntries(url.searchParams)
    expect(params).toEqual({
      client_id: clientId,
      redirect_uri: "http://app/integrations/messenger/callback",
      scope: MESSENGER_SCOPES.join(","),
      response_type: "code",
      state: expect.any(String),
      auth_type: "rerequest",
    })
    const [sessionId, nonce] = (params.state as string).split(".")
    expect(sessionId).toBe(session.id)
    expect(nonce).toMatch(/^[0-9a-f]{32,}$/)
  })

  it("refuses a 21st pending session (429 connectSessionLimitReached) until one is cancelled", async () => {
    const { api } = await withCredential("p105-oauth-limit")
    const sessions: Session[] = []
    for (let index = 0; index < 20; index += 1) {
      sessions.push((await startMessenger(api)).session)
    }
    const refused = await api.post("/v1/connections", { provider: "messenger" })
    expectError(refused, 429, "connectSessionLimitReached")
    expect((await api.delete(`/v1/connect-sessions/${sessions[0]?.id}`)).status).toBe(200)
    expect((await api.post("/v1/connections", { provider: "messenger" })).status).toBe(201)
  })
})

describe("connectSessions.get / cancel", () => {
  it("gets the session; 404 for an unknown id or another workspace's session", async () => {
    const { api } = await withCredential("p105-sess-get")
    const { api: other } = await tenantApi("p105-sess-get-other")
    const { session } = await startMessenger(api)
    const response = await api.get<Session>(`/v1/connect-sessions/${session.id}`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual(session)
    expectError(await api.get("/v1/connect-sessions/1"), 404, "notFound", {
      defined: true,
      message: "Connect session not found",
    })
    expectError(await other.get(`/v1/connect-sessions/${session.id}`), 404, "notFound")
    expectError(await other.delete(`/v1/connect-sessions/${session.id}`), 404, "notFound")
    expectError(await other.post(`/v1/connect-sessions/${session.id}/targets`, { targetIds: ["x"] }), 404, "notFound")
  })

  it("checks the session provider's kind against the token scope", async () => {
    const { tenant, api } = await withCredential("p105-sess-scope")
    const { session } = await startMessenger(api)
    const { token } = await (await fixtures()).createWorkspaceToken({
      workspaceId: tenant.workspaceId,
      name: "integrations only",
      permission: "full",
      scopes: ["integrations"],
    })
    const integrationsOnly = apiClient(token)
    for (const response of [
      await integrationsOnly.get(`/v1/connect-sessions/${session.id}`),
      await integrationsOnly.delete(`/v1/connect-sessions/${session.id}`),
      await integrationsOnly.post(`/v1/connect-sessions/${session.id}/targets`, { targetIds: ["x"] }),
    ]) {
      expectError(response, 403, "FORBIDDEN", { message: "Token is not authorized for the 'channels' scope" })
    }
    expect((await api.get<Session>(`/v1/connect-sessions/${session.id}`)).json.status).toBe("pending")
  })

  it("cancel moves an active session to cancelled; a second cancel returns it unchanged", async () => {
    const { api } = await withCredential("p105-sess-cancel")
    const { session } = await startMessenger(api)
    const cancelled = await api.delete<Session>(`/v1/connect-sessions/${session.id}`)
    expect(cancelled.status).toBe(200)
    expect(cancelled.json).toEqual({ ...session, status: "cancelled" })
    const again = await api.delete<Session>(`/v1/connect-sessions/${session.id}`)
    expect(again.status).toBe(200)
    expect(again.json.status).toBe("cancelled")
    expect((await api.get<Session>(`/v1/connect-sessions/${session.id}`)).json.status).toBe("cancelled")
    expectError(
      await api.post(`/v1/connect-sessions/${session.id}/targets`, { targetIds: ["x"] }),
      400,
      "connectSessionExpired",
    )
  })

  it("reads a session past its TTL as expired", async () => {
    const { api } = await withCredential("p105-sess-expired")
    const { session } = await startMessenger(api)
    await (await ext()).expireConnectSession({ sessionId: session.id })
    const response = await api.get<Session>(`/v1/connect-sessions/${session.id}`)
    expect(response.status).toBe(200)
    expect(response.json.status).toBe("expired")
    expect((await api.delete<Session>(`/v1/connect-sessions/${session.id}`)).json.status).toBe("expired")
  })
})

describe("connectSessions.connectTargets", () => {
  it("refuses a session that is not awaiting selection (400 connectSessionExpired) and validates targetIds", async () => {
    const { api } = await withCredential("p105-targets-pending")
    const { session } = await startMessenger(api)
    expectError(
      await api.post(`/v1/connect-sessions/${session.id}/targets`, { targetIds: ["123"] }),
      400,
      "connectSessionExpired",
      { defined: true, message: "This connect session is not awaiting target selection." },
    )
    expectInvalid(await api.post(`/v1/connect-sessions/${session.id}/targets`, { targetIds: [] }), "targetIds")
    expectInvalid(await api.post(`/v1/connect-sessions/${session.id}/targets`, {}), "targetIds")
  })
})

describe("OAuth callback round trip (messenger)", () => {
  it("a denied consent fails the session (provider_denied) and redirects to the completion page", async () => {
    const { api } = await withCredential("p105-cb-denied")
    const { session } = await startMessenger(api)
    const since = await fake.cursor()
    const response = await callback({ error: "access_denied", state: stateOf(session) })
    expectRedirect(response, `/connect/${session.id}`)
    expect(await upstreamCalls({ host: GRAPH, since })).toEqual([])
    expect((await api.get<Session>(`/v1/connect-sessions/${session.id}`)).json).toMatchObject({
      status: "failed",
      errorCode: "provider_denied",
    })
  })

  it("another provider error fails the session with provider_error", async () => {
    const { api } = await withCredential("p105-cb-error")
    const { session } = await startMessenger(api)
    expectRedirect(await callback({ error: "server_error", state: stateOf(session) }), `/connect/${session.id}`)
    expect((await api.get<Session>(`/v1/connect-sessions/${session.id}`)).json).toMatchObject({
      status: "failed",
      errorCode: "provider_error",
    })
  })

  it("redirects to an allowed redirectUrl and ignores a foreign one", async () => {
    const { api } = await withCredential("p105-cb-return")
    const allowed = (await startMessenger(api, { redirectUrl: "http://app/settings/channels?tab=messenger" })).session
    expectRedirect(await callback({ error: "access_denied", state: stateOf(allowed) }), "/settings/channels?tab=messenger")
    const foreign = (await startMessenger(api, { redirectUrl: "https://evil.parity.test/steal" })).session
    expectRedirect(await callback({ error: "access_denied", state: stateOf(foreign) }), `/connect/${foreign.id}`)
  })

  it("answers 404 for an unknown state or a state sent to another provider's callback", async () => {
    const { api } = await withCredential("p105-cb-404")
    const { session } = await startMessenger(api)
    expect((await callback({ code: "c", state: `${session.id}.${"0".repeat(64)}` })).status).toBe(404)
    expect((await callback({ code: "c", state: stateOf(session) }, "instagram")).status).toBe(404)
    expect((await api.get<Session>(`/v1/connect-sessions/${session.id}`)).json.status).toBe("pending")
  })

  it("exchanges the code, lists the pages, and connects the selected page", async () => {
    const { api } = await withCredential("p105-cb-full")
    // A page already connected in another workspace is listed but not selectable.
    const { tenant: otherTenant } = await tenantApi("p105-cb-full-other")
    const taken = newPage("Taken page")
    await (await ext()).createMessengerChannel({ workspaceId: otherTenant.workspaceId, pageId: taken.id, name: taken.name })
    const page = newPage("Parity page")
    const { session } = await startMessenger(api)
    const since = await fake.cursor()
    await scriptGraphAuthorization([page, taken])
    await scriptSubscribe()

    const code = `code-${uniqueId()}`
    const response = await callback({ code, state: stateOf(session) })
    expectRedirect(response, `/connect/${session.id}`)

    const exchange = await upstreamCalls({ host: GRAPH, since, path: /\/oauth\/access_token$/ })
    expect(exchange).toHaveLength(2)
    expect(exchange[0]?.query).toMatchObject({
      code,
      redirect_uri: "http://app/integrations/messenger/callback",
      client_id: expect.any(String),
      client_secret: expect.any(String),
    })
    expect(exchange[1]?.query).toMatchObject({ grant_type: "fb_exchange_token" })
    expect(await upstreamCalls({ host: GRAPH, since, path: /\/me\/accounts$/ })).toHaveLength(1)

    const awaiting = await api.get<Session>(`/v1/connect-sessions/${session.id}`)
    expect(awaiting.json).toMatchObject({ status: "awaiting_selection", step: "select", errorCode: null })
    expect(awaiting.json.targets).toEqual([
      { id: page.id, name: page.name, selectable: true },
      { id: taken.id, name: taken.name, selectable: false, alreadyConnected: "other_workspace" },
    ])

    const connectSince = await fake.cursor()
    const result = await api.post<TargetsResult>(`/v1/connect-sessions/${session.id}/targets`, {
      targetIds: [page.id, taken.id, "999"],
    })
    expect(result.status).toBe(200)
    const [connection] = result.json.connections
    expect(result.json.connections).toHaveLength(1)
    expect(Object.keys(connection as Connection).sort()).toEqual(CONNECTION_KEYS)
    expect(connection).toMatchObject({
      kind: "channel",
      provider: "messenger",
      channel: "messenger",
      sourceId: page.id,
      displayName: page.name,
      status: "connected",
      strategy: "oauth_redirect",
      inboxId: expect.any(String),
      integrationId: null,
    })
    expect(result.json.outcomes).toEqual([
      { targetId: page.id, status: "connected", connectionId: connection?.id },
      { targetId: taken.id, status: "duplicated", reason: "alreadyConnected" },
      { targetId: "999", status: "failed", reason: "unknown" },
    ])
    expect(result.json.session).toMatchObject({ id: session.id, connectionIds: [connection?.id] })

    const subscribe = await upstreamCalls({ host: GRAPH, since: connectSince, path: /\/me\/subscribed_apps$/ })
    expect(subscribe.length).toBeGreaterThanOrEqual(1)
    expect(subscribe[0]?.method).toBe("POST")
    expect(subscribe[0]?.headers.authorization).toBe(`Bearer ${page.token}`)

    const list = await api.get<{ data: Connection[] }>("/v1/connections")
    expect(list.json.data.map((item) => item.id)).toEqual([connection?.id])
    expect((await api.get<Session>(`/v1/connect-sessions/${session.id}`)).json.connectionIds).toEqual([connection?.id])
  })

  it("a failed code exchange fails the session with exchange_failed", async () => {
    const { api } = await withCredential("p105-cb-exchange")
    const { session } = await startMessenger(api)
    await scriptGraphAuthorization([], { exchangeStatus: 400 })
    const response = await callback({ code: "bad", state: stateOf(session) })
    expect([302, 303, 307, 308]).toContain(response.status)
    expect((await api.get<Session>(`/v1/connect-sessions/${session.id}`)).json).toMatchObject({
      status: "failed",
      errorCode: "exchange_failed",
    })
  })

  it("an account with no pages fails the session with no_candidates", async () => {
    const { api } = await withCredential("p105-cb-nopages")
    const { session } = await startMessenger(api)
    await scriptGraphAuthorization([])
    const response = await callback({ code: "c", state: stateOf(session) })
    expectRedirect(response, `/connect/${session.id}?connect_error=no_candidates`)
    expect((await api.get<Session>(`/v1/connect-sessions/${session.id}`)).json).toMatchObject({
      status: "failed",
      errorCode: "no_candidates",
    })
  })

  it("a failed webhook subscription still connects the page, as degraded", async () => {
    const { api } = await withCredential("p105-cb-degraded")
    const page = newPage("Degraded page")
    const { session } = await startMessenger(api)
    await scriptGraphAuthorization([page])
    await scriptSubscribe(500)
    expectRedirect(await callback({ code: "c", state: stateOf(session) }), `/connect/${session.id}`)
    const result = await api.post<TargetsResult>(`/v1/connect-sessions/${session.id}/targets`, { targetIds: [page.id] })
    expect(result.status).toBe(200)
    expect(result.json.connections[0]).toMatchObject({ sourceId: page.id, status: "degraded" })
    expect(result.json.outcomes).toEqual([
      {
        targetId: page.id,
        status: "connected",
        connectionId: result.json.connections[0]?.id,
        detail: expect.stringContaining("webhook subscription failed"),
      },
    ])
  })
})

describe("connections.reconnect (messenger)", () => {
  it("starts a reconnect session for the connection", async () => {
    const { tenant, api } = await withCredential("p105-reconnect")
    const page = newPage("Reconnect page")
    const channel = await (await ext()).createMessengerChannel({
      workspaceId: tenant.workspaceId,
      pageId: page.id,
      name: page.name,
    })
    const response = await api.post<Envelope>(`/v1/connections/${channel.connectionId}/reconnect`, {})
    expect(response.status).toBe(201)
    expect(response.json.connection).toBeNull()
    expect(response.json.secret).toBeNull()
    expect(response.json.session).toMatchObject({
      provider: "messenger",
      purpose: "reconnect",
      status: "pending",
      step: "authorize",
      nextAction: { type: "open_url" },
    })
    expect(normalizeUrl(response.json.session?.nextAction?.url as string)).toContain(
      "https://www.facebook.com/v23.0/dialog/oauth?",
    )
  })

  it("refuses a reconnect without a platform credential (400 connectionNotConfigured)", async () => {
    const { tenant, api } = await tenantApi("p105-reconnect-nocred")
    const page = newPage("No cred page")
    const channel = await (await ext()).createMessengerChannel({
      workspaceId: tenant.workspaceId,
      pageId: page.id,
      name: page.name,
    })
    expectError(await api.post(`/v1/connections/${channel.connectionId}/reconnect`, {}), 400, "connectionNotConfigured")
  })
})
