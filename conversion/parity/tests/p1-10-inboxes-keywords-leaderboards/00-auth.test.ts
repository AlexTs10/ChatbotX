// Auth matrix of every P1.10 operation (52): inboxes, instagramChannels,
// integrations, keywords, leaderboards.
//
// Workspace-token routes (`workspaceTokenAuthAPIForScope`): no token or an
// unknown token -> 401 INVALID_CHATBOT_TOKEN, a token without the route's
// scope -> 403 FORBIDDEN, a read-only token on a write (every non-GET) ->
// 403 FORBIDDEN, and a token with only the route's scope passes auth. Auth
// runs before input validation, so the requests carry no valid input. Ids in
// paths are numeric ids no row has; a request that passes auth answers 4xx or
// 2xx, never 5xx.

import { beforeAll, describe, expect, it } from "vitest"
import { createTenant, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { expectError } from "./helpers.ts"

type Scope = "inbox" | "channels" | "integrations" | "automation" | "leaderboards"
type Op = { id: string; method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE"; path: string; scope: Scope }

const op = (id: string, method: Op["method"], path: string, scope: Scope): Op => ({ id, method, path, scope })

const ID = "999999999999999991"
const LB = `/v1/leaderboards/${ID}`

/** Every P1.10 operation (public-spec-operations snapshot) with its token scope. */
const OPERATIONS: Op[] = [
  op("inboxes.list", "GET", "/v1/inboxes", "inbox"),
  op("inboxes.listChannels", "GET", "/v1/channels", "inbox"),
  op("inboxes.update", "PATCH", `/v1/inboxes/${ID}`, "inbox"),
  op("instagramChannels.disconnect", "DELETE", `/v1/instagram-channels/${ID}`, "channels"),
  op("instagramChannels.disconnectCapi", "DELETE", `/v1/instagram-channels/${ID}/capi`, "channels"),
  op("instagramChannels.get", "GET", `/v1/instagram-channels/${ID}`, "channels"),
  op("instagramChannels.getCoexistStatus", "GET", `/v1/instagram-channels/${ID}/coexist`, "channels"),
  op("instagramChannels.getSettings", "GET", `/v1/instagram-channels/${ID}/settings`, "channels"),
  op("instagramChannels.list", "GET", "/v1/instagram-channels", "channels"),
  op("instagramChannels.patchSettings", "PATCH", `/v1/instagram-channels/${ID}/settings`, "channels"),
  op("instagramChannels.sendCapiTestEvent", "POST", `/v1/instagram-channels/${ID}/capi/test-event`, "channels"),
  op("instagramChannels.setCapiDataset", "PUT", `/v1/instagram-channels/${ID}/capi/dataset`, "channels"),
  op("instagramChannels.setCapiTestEventCode", "PUT", `/v1/instagram-channels/${ID}/capi/test-event-code`, "channels"),
  op("instagramChannels.setCoexist", "PUT", `/v1/instagram-channels/${ID}/coexist`, "channels"),
  op("instagramChannels.updateSettings", "PUT", `/v1/instagram-channels/${ID}/settings`, "channels"),
  op("integrations.connectAiProvider", "PUT", "/v1/integrations/ai/claude", "integrations"),
  op("integrations.disconnectAiProvider", "DELETE", "/v1/integrations/ai/claude", "integrations"),
  op("integrations.get", "GET", `/v1/integrations/${ID}`, "integrations"),
  op("integrations.getAiProvider", "GET", "/v1/integrations/ai/claude", "integrations"),
  op("integrations.list", "GET", "/v1/integrations", "integrations"),
  op("integrations.tokenErrors", "GET", "/v1/integrations/status/token-errors", "integrations"),
  op("integrations.updateAiProvider", "PATCH", "/v1/integrations/ai/claude", "integrations"),
  op("keywords.create", "POST", "/v1/keywords", "automation"),
  op("keywords.delete", "DELETE", `/v1/keywords/${ID}`, "automation"),
  op("keywords.get", "GET", `/v1/keywords/${ID}`, "automation"),
  op("keywords.list", "GET", "/v1/keywords", "automation"),
  op("keywords.update", "PATCH", `/v1/keywords/${ID}`, "automation"),
  op("keywords.updateStatus", "PATCH", `/v1/keywords/${ID}/status`, "automation"),
  op("leaderboards.auditLog.list", "GET", `${LB}/audit-log`, "leaderboards"),
  op("leaderboards.create", "POST", "/v1/leaderboards", "leaderboards"),
  op("leaderboards.defaultRules", "GET", "/v1/leaderboards/default-rules", "leaderboards"),
  op("leaderboards.delete", "DELETE", LB, "leaderboards"),
  op("leaderboards.export", "POST", `${LB}/export`, "leaderboards"),
  op("leaderboards.finish", "POST", `${LB}/finish`, "leaderboards"),
  op("leaderboards.get", "GET", LB, "leaderboards"),
  op("leaderboards.getExport", "GET", `/v1/leaderboards/exports/${ID}`, "leaderboards"),
  op("leaderboards.getViewLink", "GET", `${LB}/view-link`, "leaderboards"),
  op("leaderboards.list", "GET", "/v1/leaderboards", "leaderboards"),
  op("leaderboards.participants.adjust", "POST", `${LB}/participants/${ID}/adjust`, "leaderboards"),
  op("leaderboards.participants.ban", "POST", `${LB}/participants/${ID}/ban`, "leaderboards"),
  op("leaderboards.participants.history", "GET", `${LB}/participants/${ID}/history`, "leaderboards"),
  op("leaderboards.participants.list", "GET", `${LB}/participants`, "leaderboards"),
  op("leaderboards.participants.reset", "POST", `${LB}/participants/${ID}/reset`, "leaderboards"),
  op("leaderboards.participants.unban", "POST", `${LB}/participants/${ID}/unban`, "leaderboards"),
  op("leaderboards.pause", "POST", `${LB}/pause`, "leaderboards"),
  op("leaderboards.publish", "POST", `${LB}/publish`, "leaderboards"),
  op("leaderboards.resetPoints", "POST", `${LB}/reset-points`, "leaderboards"),
  op("leaderboards.start", "POST", `${LB}/start`, "leaderboards"),
  op("leaderboards.telegramChats", "GET", "/v1/leaderboards/telegram-chats", "leaderboards"),
  op("leaderboards.top", "GET", `${LB}/top`, "leaderboards"),
  op("leaderboards.triggerRewards", "POST", `${LB}/trigger-rewards`, "leaderboards"),
  op("leaderboards.update", "PUT", LB, "leaderboards"),
]

const WRITES = OPERATIONS.filter((o) => o.method !== "GET")
const READS = OPERATIONS.filter((o) => o.method === "GET")
const SCOPES: Scope[] = ["inbox", "channels", "integrations", "automation", "leaderboards"]

const call = (token: string, o: { method: string; path: string }) => {
  const api = apiClient(token)
  const body = o.method === "GET" ? undefined : {}
  return api.http.request(o.method, `/api${o.path}`, {
    ...(token ? { bearer: token } : {}),
    noCookies: true,
    ...(body ? { json: body } : {}),
  })
}

describe("P1.10 auth matrix", () => {
  it("covers the 52 operations of the unit", () => {
    expect(OPERATIONS).toHaveLength(52)
    expect(new Set(OPERATIONS.map((o) => o.id)).size).toBe(52)
    expect(new Set(OPERATIONS.map((o) => `${o.method} ${o.path}`)).size).toBe(52)
  })

  describe("no token -> 401 INVALID_CHATBOT_TOKEN", () => {
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      expectError(await call("", o), 401, "INVALID_CHATBOT_TOKEN", {
        defined: true,
        message: "Invalid or missing workspace API token",
      })
    })
  })

  describe("unknown workspace token -> 401 INVALID_CHATBOT_TOKEN", () => {
    it.each(
      [
        "inboxes.list",
        "inboxes.update",
        "instagramChannels.list",
        "instagramChannels.setCapiDataset",
        "integrations.list",
        "integrations.connectAiProvider",
        "keywords.list",
        "keywords.create",
        "leaderboards.list",
        "leaderboards.create",
      ].map((id) => [id, OPERATIONS.find((o) => o.id === id) as Op] as const),
    )("%s", async (_id, o) => {
      expectError(await call("cbx_ws_parityUnknownTokenValue000000000000000", o), 401, "INVALID_CHATBOT_TOKEN", {
        defined: true,
      })
    })
  })

  describe("token without the operation's scope -> 403 FORBIDDEN", () => {
    let tenant: Tenant
    beforeAll(async () => {
      // `analytics` is none of the scopes this unit's operations need.
      tenant = await createTenant({ label: "p110-wrong-scope", scopes: ["analytics"] })
    })
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      expectError(await call(tenant.token, o), 403, "FORBIDDEN", {
        defined: true,
        message: `Token is not authorized for the '${o.scope}' scope`,
      })
    })
  })

  describe("a token with all other scopes but this one -> 403 FORBIDDEN", () => {
    const tenants = new Map<Scope, Tenant>()
    beforeAll(async () => {
      for (const scope of SCOPES) {
        tenants.set(
          scope,
          await createTenant({ label: `p110-not-${scope}`, scopes: SCOPES.filter((other) => other !== scope) }),
        )
      }
    })
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      expectError(await call((tenants.get(o.scope) as Tenant).token, o), 403, "FORBIDDEN", { defined: true })
    })
  })

  describe("a token with only the operation's scope passes auth", () => {
    const tenants = new Map<Scope, Tenant>()
    beforeAll(async () => {
      for (const scope of SCOPES) {
        tenants.set(scope, await createTenant({ label: `p110-scope-${scope}`, scopes: [scope] }))
      }
    })
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      const response = await call((tenants.get(o.scope) as Tenant).token, o)
      expect([401, 403], response.text).not.toContain(response.status)
      expect(response.status, response.text).toBeLessThan(500)
    })
  })

  describe("read-only token", () => {
    let tenant: Tenant
    beforeAll(async () => {
      tenant = await createTenant({ label: "p110-read-only", permission: "read_only" })
    })
    it.each(WRITES.map((o) => [o.id, o] as const))("write %s -> 403 FORBIDDEN", async (_id, o) => {
      expectError(await call(tenant.token, o), 403, "FORBIDDEN", {
        defined: true,
        message: "Read-only token cannot perform this operation",
      })
    })
    it.each(READS.map((o) => [o.id, o] as const))("read %s passes auth", async (_id, o) => {
      const response = await call(tenant.token, o)
      expect([401, 403], response.text).not.toContain(response.status)
      expect(response.status, response.text).toBeLessThan(500)
    })
  })

  it("the read-only check runs before the scope check", async () => {
    const tenant = await createTenant({ label: "p110-ro-scoped", permission: "read_only", scopes: ["analytics"] })
    const response = await call(tenant.token, { method: "POST", path: "/v1/keywords" })
    expectError(response, 403, "FORBIDDEN", { message: "Read-only token cannot perform this operation" })
  })
})
