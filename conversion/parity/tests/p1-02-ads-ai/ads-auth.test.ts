// Auth of every `ads.*` operation: no token -> 401, a token without the `ads`
// scope -> 403, a read-only token on a write -> 403. The token middleware
// and the scope middleware run before input validation, so dummy ids and
// empty bodies are enough here; per-operation behavior is in the other files.

import { beforeAll, describe, expect, it } from "vitest"
import { createTenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { comparableError } from "../../src/normalize.ts"
import { ADS_SCOPE_MESSAGE, READ_ONLY_MESSAGE } from "./ads-helpers.ts"

type Op = { id: string; method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE"; path: string; write: boolean }

const ID = "1000000000000000001"
const ops: Op[] = [
  { id: "ads.checkCampaignPrerequisites", method: "GET", path: "/v1/ads/campaigns/prerequisites", write: false },
  { id: "ads.createCampaign", method: "POST", path: "/v1/ads/campaigns", write: true },
  { id: "ads.createCampaignImageUpload", method: "POST", path: "/v1/ads/campaigns/upload-image", write: true },
  { id: "ads.createRule", method: "POST", path: "/v1/ads/conversion-rules", write: true },
  { id: "ads.deleteCampaign", method: "DELETE", path: `/v1/ads/campaigns/${ID}`, write: true },
  { id: "ads.deleteRule", method: "DELETE", path: `/v1/ads/conversion-rules/${ID}`, write: true },
  { id: "ads.disconnectConnection", method: "DELETE", path: `/v1/ads/connections/messenger/${ID}`, write: true },
  { id: "ads.getAnalyticsOverview", method: "GET", path: "/v1/ads/analytics/overview", write: false },
  { id: "ads.getAnalyticsTimeseries", method: "GET", path: "/v1/ads/analytics/timeseries", write: false },
  { id: "ads.getCampaignAdAccountDetails", method: "GET", path: "/v1/ads/campaigns/ad-accounts/act_1", write: false },
  // POST-for-read: allowed for read-only tokens (tested in ads-campaigns.test.ts).
  { id: "ads.getCampaignsInsights", method: "POST", path: "/v1/ads/campaigns/insights", write: false },
  { id: "ads.getCampaignVideoStatus", method: "GET", path: "/v1/ads/campaigns/videos/123/status", write: false },
  { id: "ads.getCapiDelivery", method: "GET", path: "/v1/ads/capi-delivery", write: false },
  { id: "ads.getConversion", method: "GET", path: `/v1/ads/conversions/${ID}`, write: false },
  { id: "ads.getFunnel", method: "GET", path: "/v1/ads/funnel", write: false },
  { id: "ads.getFunnelTimeseries", method: "GET", path: "/v1/ads/funnel/timeseries", write: false },
  { id: "ads.getRule", method: "GET", path: `/v1/ads/conversion-rules/${ID}`, write: false },
  { id: "ads.listCampaignAdAccounts", method: "GET", path: `/v1/ads/campaigns/messenger/${ID}/ad-accounts`, write: false },
  { id: "ads.listCampaignMessengerPages", method: "GET", path: "/v1/ads/campaigns/messenger-pages", write: false },
  { id: "ads.listCampaigns", method: "GET", path: "/v1/ads/campaigns", write: false },
  { id: "ads.listChannelAdAccounts", method: "GET", path: "/v1/ads/messenger/ad-accounts", write: false },
  { id: "ads.listConnections", method: "GET", path: "/v1/ads/connections", write: false },
  { id: "ads.listConversionExportRows", method: "GET", path: "/v1/ads/conversions/export", write: false },
  { id: "ads.listCustomAudiences", method: "GET", path: "/v1/ads/custom-audiences", write: false },
  { id: "ads.listReferralAds", method: "GET", path: "/v1/ads/referral-ads", write: false },
  { id: "ads.listRules", method: "GET", path: "/v1/ads/conversion-rules", write: false },
  { id: "ads.pauseCampaign", method: "POST", path: `/v1/ads/campaigns/${ID}/pause`, write: true },
  { id: "ads.publishCampaign", method: "POST", path: `/v1/ads/campaigns/${ID}/publish`, write: true },
  { id: "ads.referralStats", method: "GET", path: "/v1/ads/referral-stats", write: false },
  { id: "ads.retryCampaign", method: "POST", path: `/v1/ads/campaigns/${ID}/retry`, write: true },
  { id: "ads.startRetargetAudienceSync", method: "POST", path: "/v1/ads/retarget-audiences", write: true },
  { id: "ads.toggleRuleStatus", method: "PATCH", path: `/v1/ads/conversion-rules/${ID}/status`, write: true },
  { id: "ads.updateRule", method: "PATCH", path: `/v1/ads/conversion-rules/${ID}`, write: true },
  { id: "ads.updateRuleLegacy", method: "PUT", path: `/v1/ads/conversion-rules/${ID}`, write: true },
  { id: "ads.uploadCampaignVideo", method: "POST", path: "/v1/ads/campaigns/upload-video", write: true },
]

const call = (token: string, op: Op) => {
  const api = apiClient(token)
  const { path } = op
  switch (op.method) {
    case "GET":
      return api.get(path)
    case "DELETE":
      return api.delete(path)
    case "POST":
      return api.post(path, {})
    case "PUT":
      return api.put(path, {})
    case "PATCH":
      return api.patch(path, {})
  }
}

describe("ads: auth on every operation", () => {
  let wrongScopeToken = ""
  let readOnlyToken = ""

  beforeAll(async () => {
    wrongScopeToken = (await createTenant({ label: "ads-scope", scopes: ["contacts"] })).token
    readOnlyToken = (await createTenant({ label: "ads-ro", permission: "read_only" })).token
  })

  it("covers the 35 ads operations", () => {
    expect(new Set(ops.map((op) => op.id)).size).toBe(35)
  })

  it.each(ops)("$id without a token is 401 INVALID_CHATBOT_TOKEN", async (op) => {
    const response = await call("", op)
    expect(response.status, response.text).toBe(401)
    expect(comparableError(response.json, { keepMessage: true })).toEqual({
      defined: true,
      code: "INVALID_CHATBOT_TOKEN",
      status: 401,
      message: "Invalid or missing workspace API token",
    })
  })

  it.each(ops)("$id with an unknown token is 401", async (op) => {
    const response = await call("cbx_ws_parity-unknown-token-0000000000000000000000", op)
    expect(response.status, response.text).toBe(401)
    expect(comparableError(response.json)).toEqual({ defined: true, code: "INVALID_CHATBOT_TOKEN", status: 401 })
  })

  it.each(ops)("$id with a token without the ads scope is 403", async (op) => {
    const response = await call(wrongScopeToken, op)
    expect(response.status, response.text).toBe(403)
    expect(comparableError(response.json, { keepMessage: true })).toEqual({
      defined: true,
      code: "FORBIDDEN",
      status: 403,
      message: ADS_SCOPE_MESSAGE,
    })
  })

  it.each(ops.filter((op) => op.write))("$id with a read-only token is 403", async (op) => {
    const response = await call(readOnlyToken, op)
    expect(response.status, response.text).toBe(403)
    expect(comparableError(response.json, { keepMessage: true })).toEqual({
      defined: true,
      code: "FORBIDDEN",
      status: 403,
      message: READ_ONLY_MESSAGE,
    })
  })

  it.each(ops.filter((op) => !op.write))("$id with a read-only token passes auth", async (op) => {
    const response = await call(readOnlyToken, op)
    // Past auth: the answer is a result or an input/business error, never 401/403.
    expect([401, 403], response.text).not.toContain(response.status)
  })
})
