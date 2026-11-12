// Auth matrix of every P1.12 operation (43): miniApps, minigames, openHours,
// orders, paymentProviders.
//
// All are workspace-token routes (`workspaceTokenAuthAPIForScope`): no token
// or an unknown token -> 401 INVALID_CHATBOT_TOKEN, a token without the
// route's scope -> 403 FORBIDDEN, a read-only token on a write (every
// non-GET) -> 403 FORBIDDEN, and a token with only the route's scope passes
// auth. Auth runs before input validation, so the requests carry no valid
// input. Ids in paths are numeric ids no row has; a request that passes auth
// answers 4xx or 2xx, never 5xx, and makes no third-party call.

import { beforeAll, describe, expect, it } from "vitest"
import { createTenant, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { expectError } from "./helpers.ts"

type Scope = "mini-apps" | "minigames" | "ecommerce"
type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
type Op = { id: string; method: Method; path: string; scope: Scope; body?: unknown }

const op = (id: string, method: Method, path: string, scope: Scope, body?: unknown): Op => ({
  id,
  method,
  path,
  scope,
  body,
})

const ID = "999999999999999992"
const MA = `/v1/mini-apps/${ID}`
const MG = `/v1/minigames/${ID}`
const OR = `/v1/orders/${ID}`

/** Every operation of P1.12 (public-spec-operations snapshot) with its token scope. */
const OPERATIONS: Op[] = [
  op("miniApps.create", "POST", "/v1/mini-apps", "mini-apps"),
  op("miniApps.delete", "DELETE", MA, "mini-apps"),
  op("miniApps.deleteMany", "POST", "/v1/mini-apps/bulk-delete", "mini-apps"),
  op("miniApps.deleteWhatsappDraft", "DELETE", `${MA}/whatsapp-draft/${ID}`, "mini-apps"),
  op("miniApps.duplicate", "POST", `${MA}/duplicate`, "mini-apps"),
  op("miniApps.get", "GET", MA, "mini-apps"),
  op("miniApps.getFlowJson", "GET", `${MA}/flow-json`, "mini-apps"),
  op("miniApps.getWhatsappPreview", "GET", `${MA}/whatsapp-preview`, "mini-apps"),
  op("miniApps.list", "GET", "/v1/mini-apps", "mini-apps"),
  op("miniApps.listSubmissions", "GET", `${MA}/submissions`, "mini-apps"),
  op("miniApps.publishWhatsapp", "POST", `${MA}/publish-whatsapp`, "mini-apps"),
  op("miniApps.publishWhatsappDraft", "POST", `${MA}/whatsapp-draft/publish`, "mini-apps"),
  op("miniApps.pushWhatsappDraft", "POST", `${MA}/whatsapp-draft`, "mini-apps"),
  op("miniApps.update", "PATCH", MA, "mini-apps"),
  op("miniApps.validate", "POST", "/v1/mini-apps/validate", "mini-apps"),
  op("minigames.create", "POST", "/v1/minigames", "minigames"),
  op("minigames.delete", "DELETE", MG, "minigames"),
  op("minigames.deleteMany", "POST", "/v1/minigames/bulk-delete", "minigames"),
  op("minigames.get", "GET", MG, "minigames"),
  op("minigames.list", "GET", "/v1/minigames", "minigames"),
  op("minigames.listPlayers", "GET", `${MG}/players`, "minigames"),
  op("minigames.listPlays", "GET", `${MG}/plays`, "minigames"),
  op("minigames.setEnabled", "PATCH", `${MG}/enabled`, "minigames"),
  op("minigames.update", "PUT", MG, "minigames"),
  op("minigames.updatePartial", "PATCH", MG, "minigames"),
  op("openHours.create", "POST", "/v1/open-hours", "ecommerce"),
  op("openHours.delete", "DELETE", `/v1/open-hours/${ID}`, "ecommerce"),
  op("openHours.list", "GET", "/v1/open-hours", "ecommerce"),
  // An empty PATCH body is a 500 in the original (see 10-open-hours); send one field.
  op("openHours.update", "PATCH", `/v1/open-hours/${ID}`, "ecommerce", { name: "parity" }),
  op("orders.accept", "POST", `${OR}/accept`, "ecommerce"),
  op("orders.cancel", "POST", `${OR}/cancel`, "ecommerce"),
  op("orders.conclude", "POST", `${OR}/conclude`, "ecommerce"),
  op("orders.create", "POST", "/v1/orders", "ecommerce"),
  op("orders.delete", "DELETE", OR, "ecommerce"),
  op("orders.export", "GET", "/v1/orders/export", "ecommerce"),
  op("orders.get", "GET", OR, "ecommerce"),
  op("orders.getPaymentLink", "POST", `${OR}/payment-link`, "ecommerce"),
  op("orders.list", "GET", "/v1/orders", "ecommerce"),
  op("orders.ship", "POST", `${OR}/ship`, "ecommerce"),
  op("paymentProviders.connectStripeKey", "POST", "/v1/payment-providers/stripe/api-key", "ecommerce"),
  op("paymentProviders.disconnect", "DELETE", "/v1/payment-providers/stripe", "ecommerce"),
  op("paymentProviders.getConnectLink", "GET", "/v1/payment-providers/stripe/connect-link", "ecommerce"),
  op("paymentProviders.list", "GET", "/v1/payment-providers", "ecommerce"),
]

const WRITES = OPERATIONS.filter((o) => o.method !== "GET")
const READS = OPERATIONS.filter((o) => o.method === "GET")
const SCOPES: Scope[] = ["mini-apps", "minigames", "ecommerce"]

const call = (token: string, o: { method: string; path: string; body?: unknown }) => {
  const api = apiClient(token)
  const body = o.method === "GET" ? undefined : (o.body ?? {})
  return api.http.request(o.method, `/api${o.path}`, {
    ...(token ? { bearer: token } : {}),
    noCookies: true,
    ...(body ? { json: body } : {}),
  })
}

describe("P1.12 auth matrix", () => {
  it("covers the 43 operations of the unit", () => {
    expect(OPERATIONS).toHaveLength(43)
    expect(new Set(OPERATIONS.map((o) => o.id)).size).toBe(43)
    expect(new Set(OPERATIONS.map((o) => `${o.method} ${o.path}`)).size).toBe(43)
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
        "miniApps.list",
        "miniApps.create",
        "minigames.list",
        "minigames.update",
        "openHours.list",
        "orders.create",
        "orders.export",
        "paymentProviders.connectStripeKey",
      ].map((id) => [id, OPERATIONS.find((o) => o.id === id) as Op] as const),
    )("%s", async (_id, o) => {
      expectError(await call("cbx_ws_parityUnknownTokenValue000000000000000", o), 401, "INVALID_CHATBOT_TOKEN", {
        defined: true,
        message: "Invalid or missing workspace API token",
      })
    })
  })

  describe("token without the operation's scope -> 403 FORBIDDEN", () => {
    let tenant: Tenant
    beforeAll(async () => {
      // `analytics` is none of the scopes this unit's operations need.
      tenant = await createTenant({ label: "p112-wrong-scope", scopes: ["analytics"] })
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
          await createTenant({ label: `p112-not-${scope}`, scopes: SCOPES.filter((other) => other !== scope) }),
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
        tenants.set(scope, await createTenant({ label: `p112-scope-${scope}`, scopes: [scope] }))
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
      tenant = await createTenant({ label: "p112-read-only", permission: "read_only" })
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
    const tenant = await createTenant({ label: "p112-ro-scoped", permission: "read_only", scopes: ["analytics"] })
    const response = await call(tenant.token, { method: "POST", path: "/v1/orders" })
    expectError(response, 403, "FORBIDDEN", { message: "Read-only token cannot perform this operation" })
  })
})
