// Auth matrix of every P1.5 operation (25).
//
// Workspace-token operations (20): no token, unknown token, a token without
// the operation's scope, a token with only (one of) the operation's scope(s),
// and a read-only token (writes refused, reads let through). Auth runs before
// input validation, so the requests carry no valid input.
//
// API-channel operations (5, `channels.*`): authenticated by the channel's own
// `cbx_api_` bearer token, not by a workspace token.

import { beforeAll, describe, expect, it } from "vitest"
import { createTenant, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { expectError, tenantApi, uniqueName } from "./helpers.ts"

type Scope = "connections" | "automation" | "contacts"
type Op = { id: string; method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE"; path: string; scope: Scope }

const op = (id: string, method: Op["method"], path: string, scope: Scope): Op => ({ id, method, path, scope })

/** `connections` = the router-wide `['channels','integrations']` OR scope. */
const OPERATIONS: Op[] = [
  op("commentAutomations.getMissedCommentRun", "GET", "/v1/comment-automations/1/missed-comment-runs", "automation"),
  op("commentAutomations.getModerationSummary", "GET", "/v1/comment-automations/1/moderation-summary", "automation"),
  op("commentAutomations.listAccounts", "GET", "/v1/comment-automations/accounts", "automation"),
  op("commentAutomations.listModerationLogs", "GET", "/v1/comment-automations/1/moderation-logs", "automation"),
  op("commentAutomations.processMissedComments", "POST", "/v1/comment-automations/1/missed-comment-runs", "automation"),
  op("connectionProviders.list", "GET", "/v1/connection-providers", "connections"),
  op("connections.create", "POST", "/v1/connections", "connections"),
  op("connections.disconnect", "DELETE", "/v1/connections/1", "connections"),
  op("connections.get", "GET", "/v1/connections/1", "connections"),
  op("connections.list", "GET", "/v1/connections", "connections"),
  op("connections.reconnect", "POST", "/v1/connections/1/reconnect", "connections"),
  op("connections.refresh", "POST", "/v1/connections/1/refresh", "connections"),
  op("connections.update", "PUT", "/v1/connections/1", "connections"),
  op("connections.verify", "POST", "/v1/connections/1/verify", "connections"),
  op("connectSessions.cancel", "DELETE", "/v1/connect-sessions/1", "connections"),
  op("connectSessions.connectTargets", "POST", "/v1/connect-sessions/1/targets", "connections"),
  op("connectSessions.get", "GET", "/v1/connect-sessions/1", "connections"),
  op("contactScans.create", "POST", "/v1/contact-scans", "contacts"),
  op("contactScans.getStatus", "GET", "/v1/contact-scans/status", "contacts"),
  op("contactScans.list", "GET", "/v1/contact-scans", "contacts"),
]

const CHANNEL_OPERATIONS: Array<{ id: string; method: "GET" | "POST"; path: string }> = [
  { id: "channels.deliveryStatus", method: "POST", path: "/v1/channels/api/delivery-status" },
  { id: "channels.markRead", method: "POST", path: "/v1/channels/api/read" },
  { id: "channels.me", method: "GET", path: "/v1/channels/api/me" },
  { id: "channels.sendMessage", method: "POST", path: "/v1/channels/api/messages" },
  { id: "channels.typing", method: "POST", path: "/v1/channels/api/typing" },
]

const WRITES = OPERATIONS.filter((o) => o.method !== "GET")
const READS = OPERATIONS.filter((o) => o.method === "GET")

const SCOPE_MESSAGE: Record<Scope, string> = {
  connections: "Token is not authorized for the 'channels' or 'integrations' scope",
  automation: "Token is not authorized for the 'automation' scope",
  contacts: "Token is not authorized for the 'contacts' scope",
}

const call = (token: string, o: { method: string; path: string }) => {
  const api = apiClient(token)
  const body = o.method === "GET" || o.method === "DELETE" ? undefined : {}
  return api.http.request(o.method, `/api${o.path}`, {
    ...(token ? { bearer: token } : {}),
    noCookies: true,
    ...(body ? { json: body } : {}),
  })
}

describe("P1.5 auth matrix", () => {
  it("covers the 25 operations of the unit", () => {
    expect(OPERATIONS.length + CHANNEL_OPERATIONS.length).toBe(25)
    expect(new Set([...OPERATIONS, ...CHANNEL_OPERATIONS].map((o) => o.id)).size).toBe(25)
    expect(WRITES.map((o) => o.id).sort()).toEqual([
      "commentAutomations.processMissedComments",
      "connectSessions.cancel",
      "connectSessions.connectTargets",
      "connections.create",
      "connections.disconnect",
      "connections.reconnect",
      "connections.refresh",
      "connections.update",
      "connections.verify",
      "contactScans.create",
    ])
  })

  describe("no token -> 401 INVALID_CHATBOT_TOKEN", () => {
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      const response = await call("", o)
      expectError(response, 401, "INVALID_CHATBOT_TOKEN", {
        defined: true,
        message: "Invalid or missing workspace API token",
      })
    })
  })

  describe("unknown workspace token -> 401 INVALID_CHATBOT_TOKEN", () => {
    it.each(
      [
        "commentAutomations.listAccounts",
        "connectionProviders.list",
        "connections.create",
        "connectSessions.get",
        "contactScans.list",
      ].map((id) => [id, OPERATIONS.find((o) => o.id === id) as Op] as const),
    )("%s", async (_id, o) => {
      const response = await call("cbx_ws_parityUnknownTokenValue000000000000000", o)
      expectError(response, 401, "INVALID_CHATBOT_TOKEN", { defined: true })
    })
  })

  describe("token without the operation's scope -> 403 FORBIDDEN", () => {
    let tenant: Tenant
    beforeAll(async () => {
      // `analytics` is none of the scopes this unit's operations need.
      tenant = await createTenant({ label: "p105-wrong-scope", scopes: ["analytics"] })
    })
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      const response = await call(tenant.token, o)
      expectError(response, 403, "FORBIDDEN", { defined: true, message: SCOPE_MESSAGE[o.scope] })
    })
  })

  describe("token with only the operation's scope passes auth", () => {
    const tenants = new Map<string, Tenant>()
    beforeAll(async () => {
      for (const scope of ["channels", "integrations", "automation", "contacts"] as const) {
        tenants.set(scope, await createTenant({ label: `p105-scope-${scope}`, scopes: [scope] }))
      }
    })
    const cases = OPERATIONS.flatMap((o) =>
      o.scope === "connections"
        ? [
            [`${o.id} (channels)`, o, "channels"] as const,
            [`${o.id} (integrations)`, o, "integrations"] as const,
          ]
        : [[o.id, o, o.scope] as const],
    )
    it.each(cases)("%s", async (_id, o, scope) => {
      const response = await call((tenants.get(scope) as Tenant).token, o)
      expect([401, 403]).not.toContain(response.status)
      expect(response.status).toBeLessThan(500)
    })
  })

  describe("read-only token", () => {
    let tenant: Tenant
    beforeAll(async () => {
      tenant = await createTenant({ label: "p105-read-only", permission: "read_only" })
    })
    it.each(WRITES.map((o) => [o.id, o] as const))("write %s -> 403 FORBIDDEN", async (_id, o) => {
      const response = await call(tenant.token, o)
      expectError(response, 403, "FORBIDDEN", {
        defined: true,
        message: "Read-only token cannot perform this operation",
      })
    })
    it.each(READS.map((o) => [o.id, o] as const))("read %s passes auth", async (_id, o) => {
      const response = await call(tenant.token, o)
      expect([401, 403]).not.toContain(response.status)
      expect(response.status).toBeLessThan(500)
    })
  })
})

describe("P1.5 API channel token auth (channels.*)", () => {
  let workspaceToken: string
  let channelToken: string

  beforeAll(async () => {
    const { tenant, api } = await tenantApi("p105-chan-auth")
    workspaceToken = tenant.token
    const created = await api.post<{ secret: { token: string } }>("/v1/connections", {
      provider: "api",
      config: { name: uniqueName("auth").slice(0, 40) },
    })
    expect(created.status).toBe(201)
    channelToken = created.json.secret.token
  })

  it.each(CHANNEL_OPERATIONS.map((o) => [o.id, o] as const))("no token -> 401 UNAUTHORIZED: %s", async (_id, o) => {
    const response = await call("", o)
    expectError(response, 401, "UNAUTHORIZED", { defined: true })
  })

  it.each(CHANNEL_OPERATIONS.map((o) => [o.id, o] as const))(
    "unknown cbx_api_ token -> 401 UNAUTHORIZED: %s",
    async (_id, o) => {
      const response = await call("cbx_api_parityUnknownChannelToken0000000000000", o)
      expectError(response, 401, "UNAUTHORIZED", { defined: true })
    },
  )

  it.each(CHANNEL_OPERATIONS.map((o) => [o.id, o] as const))(
    "a workspace token is not a channel token -> 401 UNAUTHORIZED: %s",
    async (_id, o) => {
      const response = await call(workspaceToken, o)
      expectError(response, 401, "UNAUTHORIZED", { defined: true })
    },
  )

  it.each(CHANNEL_OPERATIONS.map((o) => [o.id, o] as const))(
    "the channel token passes auth: %s",
    async (_id, o) => {
      const response = await call(channelToken, o)
      expect([401, 403]).not.toContain(response.status)
      expect(response.status).toBeLessThan(500)
    },
  )

  it("the channel token is not a workspace token (workspace route -> 401 INVALID_CHATBOT_TOKEN)", async () => {
    const response = await call(channelToken, { method: "GET", path: "/v1/connections" })
    expectError(response, 401, "INVALID_CHATBOT_TOKEN", { defined: true })
  })
})
