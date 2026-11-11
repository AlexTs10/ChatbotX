// Auth for every P1.8 operation: no token -> 401, unknown token -> 401, a
// token without the operation's scope -> 403, a read-only token on a write ->
// 403, and a read-only token on a read passes auth. The auth middlewares run
// before input validation, so placeholder ids are enough here.

import { beforeAll, describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { comparableError } from "../../src/normalize.ts"
import { extraToken, tenantApi } from "./helpers.ts"

type Op = {
  id: string
  method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
  path: string
  scope: string
}

const op = (id: string, method: Op["method"], path: string, scope: string): Op => ({ id, method, path, scope })

/** Every operation in scope of P1.8 (public-spec-operations snapshot). */
export const P108_OPERATIONS: Op[] = [
  op("dynamicImages.create", "POST", "/v1/dynamic-images", "media"),
  op("dynamicImages.delete", "DELETE", "/v1/dynamic-images/1", "media"),
  op("dynamicImages.get", "GET", "/v1/dynamic-images/1", "media"),
  op("dynamicImages.list", "GET", "/v1/dynamic-images", "media"),
  op("dynamicImages.setEnabled", "PATCH", "/v1/dynamic-images/1/enabled", "media"),
  op("dynamicImages.update", "PUT", "/v1/dynamic-images/1", "media"),
  op("ecommerceAnalytics.get", "GET", "/v1/ecommerce/analytics", "ecommerce"),
  op("ecommerceSettings.get", "GET", "/v1/ecommerce/settings", "ecommerce"),
  op("ecommerceSettings.update", "PATCH", "/v1/ecommerce/settings", "ecommerce"),
  op("emailTopics.create", "POST", "/v1/email-topics", "broadcasts"),
  op("emailTopics.delete", "DELETE", "/v1/email-topics/1", "broadcasts"),
  op("emailTopics.get", "GET", "/v1/email-topics/1", "broadcasts"),
  op("emailTopics.list", "GET", "/v1/email-topics", "broadcasts"),
  op("emailTopics.update", "PUT", "/v1/email-topics/1", "broadcasts"),
  op("errorLogs.list", "GET", "/v1/error-logs", "analytics"),
  op("externalWebhooks.create", "POST", "/v1/external-webhooks", "integrations"),
  op("externalWebhooks.delete", "DELETE", "/v1/external-webhooks/1", "integrations"),
  op("externalWebhooks.list", "GET", "/v1/external-webhooks", "integrations"),
  op("facebookLeadAds.create", "POST", "/v1/facebook-lead-ads", "automation"),
  op("facebookLeadAds.delete", "DELETE", "/v1/facebook-lead-ads/1", "automation"),
  op("facebookLeadAds.get", "GET", "/v1/facebook-lead-ads/1", "automation"),
  op("facebookLeadAds.list", "GET", "/v1/facebook-lead-ads", "automation"),
  op("facebookLeadAds.listForms", "GET", "/v1/facebook-lead-ads/forms", "automation"),
  op("facebookLeadAds.listPages", "GET", "/v1/facebook-lead-ads/pages", "automation"),
  op("facebookLeadAds.update", "PATCH", "/v1/facebook-lead-ads/1", "automation"),
  op("facebookMarketingMessages.adAccounts.list", "GET", "/v1/marketing-messages/ad-accounts", "ads"),
  op("facebookMarketingMessages.audiences.create", "POST", "/v1/marketing-messages/audiences", "ads"),
  op("facebookMarketingMessages.audiences.delete", "DELETE", "/v1/marketing-messages/audiences/1", "ads"),
  op("facebookMarketingMessages.audiences.list", "GET", "/v1/marketing-messages/audiences", "ads"),
  op("facebookMarketingMessages.audiences.liveStatus", "GET", "/v1/marketing-messages/audiences/live", "ads"),
  op("facebookMarketingMessages.audiences.uploadMore", "POST", "/v1/marketing-messages/audiences/1/rows", "ads"),
  op("facebookMarketingMessages.campaigns.create", "POST", "/v1/marketing-messages/campaigns", "ads"),
  op("facebookMarketingMessages.campaigns.delete", "DELETE", "/v1/marketing-messages/campaigns/1", "ads"),
  op("facebookMarketingMessages.campaigns.get", "GET", "/v1/marketing-messages/campaigns/1", "ads"),
  op("facebookMarketingMessages.campaigns.list", "GET", "/v1/marketing-messages/campaigns", "ads"),
  op("facebookMarketingMessages.campaigns.resume", "POST", "/v1/marketing-messages/campaigns/1/resume", "ads"),
  op("facebookMarketingMessages.campaigns.send", "POST", "/v1/marketing-messages/campaigns/1/send", "ads"),
  op("facebookMarketingMessages.campaigns.stop", "POST", "/v1/marketing-messages/campaigns/1/stop", "ads"),
  op("facebookMarketingMessages.campaigns.update", "PUT", "/v1/marketing-messages/campaigns/1", "ads"),
  op("facebookMarketingMessages.pages.list", "GET", "/v1/marketing-messages/pages", "ads"),
  op("facebookMarketingMessages.statistics.get", "GET", "/v1/marketing-messages/statistics", "ads"),
  op("fbComments.create", "POST", "/v1/fb-comments", "automation"),
  op("fbComments.delete", "DELETE", "/v1/fb-comments/1", "automation"),
  op("fbComments.deleteMany", "POST", "/v1/fb-comments/bulk-delete", "automation"),
  op("fbComments.get", "GET", "/v1/fb-comments/1", "automation"),
  op("fbComments.list", "GET", "/v1/fb-comments", "automation"),
  op("fbComments.listPosts", "GET", "/v1/fb-comments/facebook-posts", "automation"),
  op("fbComments.update", "PATCH", "/v1/fb-comments/1", "automation"),
]

const call = (token: string, entry: Op) => {
  const api = apiClient(token)
  switch (entry.method) {
    case "GET":
      return api.get(entry.path)
    case "DELETE":
      return api.delete(entry.path)
    case "POST":
      return api.post(entry.path, {})
    case "PUT":
      return api.put(entry.path, {})
    case "PATCH":
      return api.patch(entry.path, {})
  }
}

describe("P1.8 auth: every operation", () => {
  // One workspace per token kind: the API allows 120 calls per 10 s per
  // workspace, and each table below makes up to 48.
  let tenant: Tenant
  let readOnlyToken: string
  let wrongScopeToken: string

  beforeAll(async () => {
    ;({ tenant } = await tenantApi("p108-auth"))
    const { tenant: readOnlyTenant } = await tenantApi("p108-auth-ro")
    readOnlyToken = await extraToken(readOnlyTenant, { permission: "read_only" })
    const { tenant: scopedTenant } = await tenantApi("p108-auth-scope")
    // `minigames` is not the scope of any P1.8 operation.
    wrongScopeToken = await extraToken(scopedTenant, { scopes: ["minigames"] })
  })

  it("lists 48 operations", () => {
    expect(P108_OPERATIONS).toHaveLength(48)
    expect(new Set(P108_OPERATIONS.map((entry) => entry.id)).size).toBe(48)
  })

  it.each(P108_OPERATIONS)("$id without a token is 401 INVALID_CHATBOT_TOKEN", async (entry) => {
    const response = await call("", entry)
    expect(response.status).toBe(401)
    expect(comparableError(response.json, { keepMessage: true })).toEqual({
      defined: true,
      code: "INVALID_CHATBOT_TOKEN",
      status: 401,
      message: "Invalid or missing workspace API token",
    })
  })

  it.each(P108_OPERATIONS)("$id with an unknown token is 401", async (entry) => {
    const response = await call("cbx_ws_parity-unknown-token-000000000000000000000000", entry)
    expect(response.status).toBe(401)
    expect((response.json as { code?: string }).code).toBe("INVALID_CHATBOT_TOKEN")
  })

  it.each(P108_OPERATIONS)("$id with a token without its scope is 403", async (entry) => {
    const response = await call(wrongScopeToken, entry)
    expect(response.status).toBe(403)
    expect(comparableError(response.json, { keepMessage: true })).toEqual({
      defined: true,
      code: "FORBIDDEN",
      status: 403,
      message: `Token is not authorized for the '${entry.scope}' scope`,
    })
  })

  const writes = P108_OPERATIONS.filter((entry) => entry.method !== "GET")
  const reads = P108_OPERATIONS.filter((entry) => entry.method === "GET")

  it.each(writes)("$id with a read-only token is 403", async (entry) => {
    const response = await call(readOnlyToken, entry)
    expect(response.status).toBe(403)
    expect(comparableError(response.json, { keepMessage: true })).toEqual({
      defined: true,
      code: "FORBIDDEN",
      status: 403,
      message: "Read-only token cannot perform this operation",
    })
  })

  it.each(reads)("$id with a read-only token passes auth", async (entry) => {
    const response = await call(readOnlyToken, entry)
    expect([401, 403]).not.toContain(response.status)
  })

  it.each([
    ["media", "/v1/dynamic-images"],
    ["ecommerce", "/v1/ecommerce/settings"],
    ["broadcasts", "/v1/email-topics"],
    ["analytics", "/v1/error-logs"],
    ["integrations", "/v1/external-webhooks"],
    ["automation", "/v1/facebook-lead-ads"],
    ["automation", "/v1/fb-comments"],
    ["ads", "/v1/marketing-messages/campaigns"],
  ])("a token with only the %s scope may read %s", async (scope, path) => {
    const token = await extraToken(tenant, { scopes: [scope] })
    const response = await apiClient(token).get(path)
    expect(response.status, response.text).toBe(200)
  })
})
