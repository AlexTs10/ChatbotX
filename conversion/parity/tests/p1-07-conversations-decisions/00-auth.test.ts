// Auth matrix of every P1.7 operation (53): no token, unknown token, a token
// without the operation's scope, a token with only that scope, and a
// read-only token (writes refused, reads let through). Auth runs before input
// validation, so the requests carry placeholder ids and no valid input.

import { beforeAll, describe, expect, it } from "vitest"
import { createTenant, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { expectError } from "./helpers.ts"

type Scope = "inbox" | "ecommerce" | "contacts" | "automation"
type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
type Op = { id: string; method: Method; path: string; scope: Scope }

const op = (id: string, method: Method, path: string, scope: Scope): Op => ({ id, method, path, scope })

const CV = "/v1/conversations/1"
const CT = "/v1/coupon-topics/1"
const DC = "/v1/discount-codes"

export const OPERATIONS: Op[] = [
  op("conversations.archive", "POST", `${CV}/archive`, "inbox"),
  op("conversations.assign", "POST", `${CV}/assign`, "inbox"),
  op("conversations.disableBot", "POST", `${CV}/disable-bot`, "inbox"),
  op("conversations.enableBot", "POST", `${CV}/enable-bot`, "inbox"),
  op("conversations.follow", "POST", `${CV}/follow`, "inbox"),
  op("conversations.get", "GET", CV, "inbox"),
  op("conversations.list", "GET", "/v1/conversations", "inbox"),
  op("conversations.read", "POST", `${CV}/read`, "inbox"),
  op("conversations.threadControl", "POST", `${CV}/thread-control`, "inbox"),
  op("conversations.unarchive", "POST", `${CV}/unarchive`, "inbox"),
  op("conversations.unfollow", "POST", `${CV}/unfollow`, "inbox"),
  op("conversations.unread", "POST", `${CV}/unread`, "inbox"),
  op("coupons.archiveTopic", "POST", `${CT}/archive`, "ecommerce"),
  op("coupons.bulkCreate", "POST", `${CT}/coupons/bulk`, "ecommerce"),
  op("coupons.createImportUploadUrl", "POST", "/v1/coupon-imports/upload-url", "ecommerce"),
  op("coupons.createTopic", "POST", "/v1/coupon-topics", "ecommerce"),
  op("coupons.deleteTopic", "DELETE", CT, "ecommerce"),
  op("coupons.export", "POST", "/v1/coupon-exports", "ecommerce"),
  op("coupons.getExport", "GET", "/v1/coupon-exports/1", "ecommerce"),
  op("coupons.getTopic", "GET", CT, "ecommerce"),
  op("coupons.import", "POST", "/v1/coupon-imports", "ecommerce"),
  op("coupons.issueCoupon", "POST", `${CT}/issue`, "ecommerce"),
  op("coupons.listContactCoupons", "GET", "/v1/contacts/id%3A1/coupons", "ecommerce"),
  op("coupons.listCoupons", "GET", "/v1/coupons", "ecommerce"),
  op("coupons.listTopics", "GET", "/v1/coupon-topics", "ecommerce"),
  op("coupons.markCouponUsed", "POST", `${CT}/mark-used`, "ecommerce"),
  op("coupons.unarchiveTopic", "POST", `${CT}/unarchive`, "ecommerce"),
  op("coupons.updateTopic", "PUT", CT, "ecommerce"),
  op("customFields.create", "POST", "/v1/custom-fields", "contacts"),
  op("customFields.delete", "DELETE", "/v1/custom-fields/1", "contacts"),
  op("customFields.get", "GET", "/v1/custom-fields/1", "contacts"),
  op("customFields.list", "GET", "/v1/custom-fields", "contacts"),
  op("customFields.update", "PUT", "/v1/custom-fields/1", "contacts"),
  op("decisionConnections.create", "POST", "/v1/decision-connections", "automation"),
  op("decisionConnections.list", "GET", "/v1/decision-connections", "automation"),
  op("decisionConnections.setEnabled", "PATCH", "/v1/decision-connections/1/status", "automation"),
  op("decisionConnections.test", "POST", "/v1/decision-connections/1/test", "automation"),
  op("decisionConnections.update", "PUT", "/v1/decision-connections/1", "automation"),
  op("decisionProfiles.create", "POST", "/v1/decision-profiles", "automation"),
  op("decisionProfiles.deleteMany", "DELETE", "/v1/decision-profiles", "automation"),
  op("decisionProfiles.get", "GET", "/v1/decision-profiles/1", "automation"),
  op("decisionProfiles.list", "GET", "/v1/decision-profiles", "automation"),
  op(
    "decisionProfiles.listGlobalRouterDestinations",
    "GET",
    "/v1/decision-profiles/global-router-destinations",
    "automation",
  ),
  op("decisionProfiles.setEnabled", "PATCH", "/v1/decision-profiles/1/status", "automation"),
  op("decisionProfiles.update", "PUT", "/v1/decision-profiles/1", "automation"),
  op("discountCodes.create", "POST", DC, "ecommerce"),
  op("discountCodes.delete", "DELETE", `${DC}/1`, "ecommerce"),
  op("discountCodes.generateCode", "GET", `${DC}/generate-code`, "ecommerce"),
  op("discountCodes.get", "GET", `${DC}/1`, "ecommerce"),
  op("discountCodes.list", "GET", DC, "ecommerce"),
  op("discountCodes.setActive", "PATCH", `${DC}/1/active`, "ecommerce"),
  op("discountCodes.update", "PATCH", `${DC}/1`, "ecommerce"),
  op("discountCodes.validate", "POST", `${DC}/validate`, "ecommerce"),
]

// A read-only token may GET; none of this unit's POST routes is on the
// POST-for-read allow-list (`discountCodes.validate` and
// `decisionConnections.test` are refused too).
const READS = OPERATIONS.filter((o) => o.method === "GET")
const WRITES = OPERATIONS.filter((o) => o.method !== "GET")
const SCOPES: Scope[] = ["inbox", "ecommerce", "contacts", "automation"]

const call = (token: string, o: Op) => {
  const api = apiClient(token)
  return api.http.request(o.method, `/api${o.path}`, {
    ...(token ? { bearer: token } : {}),
    noCookies: true,
    ...(o.method === "GET" ? {} : { json: {} }),
  })
}

describe("P1.7 auth matrix", () => {
  it("covers the 53 operations of the unit", () => {
    expect(OPERATIONS).toHaveLength(53)
    expect(new Set(OPERATIONS.map((o) => o.id)).size).toBe(53)
    expect(new Set(OPERATIONS.map((o) => `${o.method} ${o.path}`)).size).toBe(53)
    expect(READS).toHaveLength(16)
    expect(WRITES).toHaveLength(37)
    const count = (scope: Scope) => OPERATIONS.filter((o) => o.scope === scope).length
    expect({
      inbox: count("inbox"),
      ecommerce: count("ecommerce"),
      contacts: count("contacts"),
      automation: count("automation"),
    }).toEqual({ inbox: 12, ecommerce: 24, contacts: 5, automation: 12 })
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
        "conversations.list",
        "conversations.assign",
        "coupons.createTopic",
        "customFields.list",
        "decisionConnections.test",
        "discountCodes.validate",
      ].map((id) => [id, OPERATIONS.find((o) => o.id === id) as Op] as const),
    )("%s", async (_id, o) => {
      const response = await call("cbx_ws_parityUnknownTokenValue000000000000000", o)
      expectError(response, 401, "INVALID_CHATBOT_TOKEN", { defined: true })
    })
  })

  describe("token without the operation's scope -> 403 FORBIDDEN", () => {
    let tenant: Tenant
    beforeAll(async () => {
      // `analytics` is none of the four scopes this unit's operations need.
      tenant = await createTenant({ label: "p107-wrong-scope", scopes: ["analytics"] })
    })
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      const response = await call(tenant.token, o)
      expectError(response, 403, "FORBIDDEN", {
        defined: true,
        message: `Token is not authorized for the '${o.scope}' scope`,
      })
    })
  })

  describe("a token with the other three scopes is still refused", () => {
    const tenants = new Map<Scope, Tenant>()
    beforeAll(async () => {
      for (const scope of SCOPES) {
        tenants.set(
          scope,
          await createTenant({ label: `p107-not-${scope}`, scopes: SCOPES.filter((s) => s !== scope) }),
        )
      }
    })
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      const response = await call((tenants.get(o.scope) as Tenant).token, o)
      expectError(response, 403, "FORBIDDEN", {
        message: `Token is not authorized for the '${o.scope}' scope`,
      })
    })
  })

  describe("token with only the operation's scope passes auth", () => {
    const tenants = new Map<Scope, Tenant>()
    beforeAll(async () => {
      for (const scope of SCOPES) {
        tenants.set(scope, await createTenant({ label: `p107-scope-${scope}`, scopes: [scope] }))
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
      tenant = await createTenant({ label: "p107-read-only", permission: "read_only" })
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
      expect([401, 403], response.text).not.toContain(response.status)
      expect(response.status, response.text).toBeLessThan(500)
    })
  })
})
