// Auth matrix of every P1.9 operation (53): flows, folders, googleAds,
// igComments, igStories, inboundWebhookChannels, inboundWebhooks, inboxTeams.
//
// Workspace-token routes (`workspaceTokenAuthAPIForScope`): no token or an
// unknown token -> 401 INVALID_CHATBOT_TOKEN, a token without the route's
// scope -> 403 FORBIDDEN, a read-only token on a write (every non-GET, also
// the read-shaped POSTs `flows.validate` and `inboundWebhooks.previewMapping`)
// -> 403 FORBIDDEN, and a token with only the route's scope passes auth.
// Auth runs before input validation, so the requests carry no valid input.

import { beforeAll, describe, expect, it } from "vitest"
import { createTenant, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { expectError } from "./helpers.ts"

type Scope = "automation" | "contacts" | "ads" | "integrations" | "inbox"
type Op = { id: string; method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE"; path: string; scope: Scope }

const op = (id: string, method: Op["method"], path: string, scope: Scope): Op => ({ id, method, path, scope })

/** Every P1.9 operation (public-spec-operations snapshot) with its token scope. */
const OPERATIONS: Op[] = [
  op("flows.create", "POST", "/v1/flows", "automation"),
  op("flows.delete", "DELETE", "/v1/flows/1", "automation"),
  op("flows.duplicate", "POST", "/v1/flows/1/duplicate", "automation"),
  op("flows.export", "GET", "/v1/flows/1/export", "automation"),
  op("flows.get", "GET", "/v1/flows/1", "automation"),
  op("flows.import", "POST", "/v1/flows/import", "automation"),
  op("flows.list", "GET", "/v1/flows", "automation"),
  op("flows.listNodes", "GET", "/v1/flows/1/nodes", "automation"),
  op("flows.publish", "POST", "/v1/flows/1/publish", "automation"),
  op("flows.update", "PATCH", "/v1/flows/1", "automation"),
  op("flows.updateDraft", "PUT", "/v1/flows/1/draft", "automation"),
  op("flows.validate", "POST", "/v1/flows/validate", "automation"),
  op("flows.versions", "GET", "/v1/flows/1/versions", "automation"),
  op("folders.create", "POST", "/v1/folders", "contacts"),
  op("folders.delete", "DELETE", "/v1/folders/1", "contacts"),
  op("folders.list", "GET", "/v1/folders", "contacts"),
  op("folders.update", "PUT", "/v1/folders/1", "contacts"),
  op("googleAds.getConnection", "GET", "/v1/google-ads/connection", "ads"),
  op("googleAds.getStats", "GET", "/v1/google-ads/stats", "ads"),
  op("googleAds.listEvents", "GET", "/v1/google-ads/events", "ads"),
  op("igComments.create", "POST", "/v1/ig-comments", "automation"),
  op("igComments.delete", "DELETE", "/v1/ig-comments/1", "automation"),
  op("igComments.deleteMany", "POST", "/v1/ig-comments/bulk-delete", "automation"),
  op("igComments.get", "GET", "/v1/ig-comments/1", "automation"),
  op("igComments.list", "GET", "/v1/ig-comments", "automation"),
  op("igComments.listMedia", "GET", "/v1/ig-comments/instagram-media", "automation"),
  op("igComments.update", "PATCH", "/v1/ig-comments/1", "automation"),
  op("igStories.create", "POST", "/v1/ig-stories", "automation"),
  op("igStories.delete", "DELETE", "/v1/ig-stories/1", "automation"),
  op("igStories.deleteMany", "POST", "/v1/ig-stories/bulk-delete", "automation"),
  op("igStories.get", "GET", "/v1/ig-stories/1", "automation"),
  op("igStories.list", "GET", "/v1/ig-stories", "automation"),
  op("igStories.listStories", "GET", "/v1/ig-stories/instagram-stories", "automation"),
  op("igStories.update", "PATCH", "/v1/ig-stories/1", "automation"),
  op("inboundWebhookChannels.list", "GET", "/v1/inbound-webhook-channels", "integrations"),
  op("inboundWebhooks.create", "POST", "/v1/inbound-webhooks", "integrations"),
  op("inboundWebhooks.delete", "DELETE", "/v1/inbound-webhooks/1", "integrations"),
  op("inboundWebhooks.deleteMany", "POST", "/v1/inbound-webhooks/bulk-delete", "integrations"),
  op("inboundWebhooks.get", "GET", "/v1/inbound-webhooks/1", "integrations"),
  op("inboundWebhooks.getRequest", "GET", "/v1/inbound-webhooks/1/requests/1", "integrations"),
  op("inboundWebhooks.list", "GET", "/v1/inbound-webhooks", "integrations"),
  op("inboundWebhooks.listRequests", "GET", "/v1/inbound-webhooks/1/requests", "integrations"),
  op("inboundWebhooks.previewMapping", "POST", "/v1/inbound-webhooks/preview-mapping", "integrations"),
  op("inboundWebhooks.regenerateToken", "POST", "/v1/inbound-webhooks/1/regenerate-token", "integrations"),
  op("inboundWebhooks.update", "PUT", "/v1/inbound-webhooks/1", "integrations"),
  op("inboundWebhooks.updateSettings", "PATCH", "/v1/inbound-webhooks/1/settings", "integrations"),
  op("inboxTeams.addMembers", "POST", "/v1/teams/1/members", "inbox"),
  op("inboxTeams.create", "POST", "/v1/teams", "inbox"),
  op("inboxTeams.delete", "DELETE", "/v1/teams/1", "inbox"),
  op("inboxTeams.get", "GET", "/v1/teams/1", "inbox"),
  op("inboxTeams.list", "GET", "/v1/teams", "inbox"),
  op("inboxTeams.removeMembers", "DELETE", "/v1/teams/1/members", "inbox"),
  op("inboxTeams.update", "PATCH", "/v1/teams/1", "inbox"),
]

const WRITES = OPERATIONS.filter((o) => o.method !== "GET")
const READS = OPERATIONS.filter((o) => o.method === "GET")
const SCOPES: Scope[] = ["automation", "contacts", "ads", "integrations", "inbox"]

const call = (token: string, o: { method: string; path: string }) => {
  const api = apiClient(token)
  const body = o.method === "GET" ? undefined : {}
  return api.http.request(o.method, `/api${o.path}`, {
    ...(token ? { bearer: token } : {}),
    noCookies: true,
    ...(body ? { json: body } : {}),
  })
}

describe("P1.9 auth matrix", () => {
  it("covers the 53 operations of the unit", () => {
    expect(OPERATIONS).toHaveLength(53)
    expect(new Set(OPERATIONS.map((o) => o.id)).size).toBe(53)
    expect(new Set(OPERATIONS.map((o) => `${o.method} ${o.path}`)).size).toBe(53)
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
        "flows.list",
        "flows.create",
        "folders.list",
        "googleAds.getConnection",
        "igComments.list",
        "igStories.create",
        "inboundWebhookChannels.list",
        "inboundWebhooks.create",
        "inboxTeams.list",
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
      tenant = await createTenant({ label: "p109-wrong-scope", scopes: ["analytics"] })
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
          await createTenant({ label: `p109-not-${scope}`, scopes: SCOPES.filter((other) => other !== scope) }),
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
        tenants.set(scope, await createTenant({ label: `p109-scope-${scope}`, scopes: [scope] }))
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
      tenant = await createTenant({ label: "p109-read-only", permission: "read_only" })
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

  it("accepts the deprecated ?token= query parameter", async () => {
    const tenant = await createTenant({ label: "p109-query-token" })
    const response = await apiClient("").http.get("/api/v1/flows", { query: { token: tenant.token }, noCookies: true })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ data: [], pageCount: 0 })
  })
})
