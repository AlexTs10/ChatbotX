// Ads reports and audiences that read Meta (fake Graph):
// ads.getAnalyticsOverview, ads.getAnalyticsTimeseries,
// ads.listChannelAdAccounts, ads.listCustomAudiences,
// ads.startRetargetAudienceSync (worker job -> Graph audience users).

import { createHash } from "node:crypto"
import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { createTenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  adsFixtures,
  expectError,
  expectValidationIssue,
  GRAPH,
  graphCalls,
  graphScript,
  jsonBody,
  messengerAdsSetup,
  uniqueAccessToken,
  uniqueAdAccountId,
  uniqueDigits,
} from "./ads-helpers.ts"

const DAYS = { from: "2026-10-01", to: "2026-10-07" }

/** Two ad-referred contacts on ad 120001, one lead and one purchase (30 USD). */
const seedFunnel = async (workspaceId: string, page: { integrationId: string; inboxId: string }) => {
  const ext = await adsFixtures()
  const referral = { adId: "120001", source: "ADS" }
  const contact = (n: number, at: string) =>
    ext.createContactInbox({
      workspaceId,
      inboxId: page.inboxId,
      channel: "messenger",
      sourceId: `psid-${n}-${uniqueId()}`,
      firstName: `Buyer${n}`,
      email: `buyer${n}-${uniqueId()}@parity.test`,
      firstInteractionAt: at,
      referral,
    })
  const c1 = await contact(1, "2026-10-02T09:00:00.000Z")
  const c2 = await contact(2, "2026-10-03T09:00:00.000Z")
  const base = { workspaceId, channel: "messenger" as const, integrationId: page.integrationId, adId: "120001" }
  await ext.createConversionEvent({
    ...base,
    eventType: "lead",
    occurredAt: "2026-10-02T10:00:00.000Z",
    sourceEventId: `lead-${uniqueId()}`,
    contactInboxId: c1.contactInboxId,
  })
  await ext.createConversionEvent({
    ...base,
    eventType: "purchase",
    occurredAt: "2026-10-03T10:00:00.000Z",
    sourceEventId: `purchase-${uniqueId()}`,
    contactInboxId: c2.contactInboxId,
    currency: "USD",
    value: "30",
  })
  return { c1, c2 }
}

describe("ads.getAnalyticsOverview (GET /v1/ads/analytics/overview)", () => {
  it("merges the DB funnel with Meta spend of the connection's ad accounts", async () => {
    const setup = await messengerAdsSetup({ label: "analytics-overview" })
    await seedFunnel(setup.tenant.workspaceId, setup)
    const adAccountId = uniqueAdAccountId()
    const since = await fake.cursor()
    await graphScript("GET", "/me/adaccounts", { status: 200, json: { data: [{ id: adAccountId, name: "Main" }] } })
    await graphScript("GET", `/${adAccountId}/insights`, {
      status: 200,
      json: {
        data: [
          { ad_id: "120001", ad_name: "Ad One", account_currency: "USD", spend: "20", impressions: "1000", clicks: "50" },
          { ad_id: "777", ad_name: "Spend only", account_currency: "USD", spend: "5", impressions: "100", clicks: "0" },
        ],
      },
    })

    const response = await setup.api.get("/v1/ads/analytics/overview", { query: { ...DAYS, channel: "messenger" } })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      totals: {
        conversations: 2,
        leads: 1,
        purchases: 1,
        revenue: 30,
        spend: 25,
        costPerLead: 25,
        costPerPurchase: 25,
        roas: 30 / 25,
        impressions: 1100,
        clicks: 50,
        cpc: 25 / 50,
        ctr: 50 / 1100,
        cpm: (25 / 1100) * 1000,
        costPerConversation: 25 / 2,
      },
      perAd: [
        {
          adId: "120001",
          adName: "Ad One",
          conversations: 2,
          leads: 1,
          purchases: 1,
          revenue: 30,
          spend: 20,
          costPerLead: 20,
          costPerPurchase: 20,
          roas: 30 / 20,
          impressions: 1000,
          clicks: 50,
          cpc: 20 / 50,
          ctr: 50 / 1000,
          cpm: (20 / 1000) * 1000,
          costPerConversation: 20 / 2,
        },
        {
          adId: "777",
          adName: "Spend only",
          conversations: 0,
          leads: 0,
          purchases: 0,
          revenue: 0,
          spend: 5,
          costPerLead: null,
          costPerPurchase: null,
          roas: 0,
          impressions: 100,
          clicks: 0,
          cpc: null,
          ctr: 0,
          cpm: (5 / 100) * 1000,
          costPerConversation: null,
        },
      ],
      spendCurrency: "USD",
    })

    const calls = await graphCalls(setup.accessToken, since)
    expect(calls.map((call) => ({ path: call.path, query: call.query }))).toEqual([
      { path: "/v23.0/me/adaccounts", query: { fields: "id,name", limit: "499", access_token: setup.accessToken } },
      {
        path: `/v23.0/${adAccountId}/insights`,
        query: {
          level: "ad",
          fields: "ad_id,ad_name,account_currency,spend,impressions,clicks",
          time_range: JSON.stringify({ since: "2026-10-01", until: "2026-10-07" }),
          limit: "499",
          access_token: setup.accessToken,
        },
      },
    ])
  })

  it("is funnel-only (no Graph call) without an ads-eligible channel or connection", async () => {
    const tenant = await createTenant({ label: "analytics-empty" })
    const since = await fake.cursor()
    const response = await apiClient(tenant.token).get("/v1/ads/analytics/overview", { query: DAYS })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      totals: {
        conversations: 0,
        leads: 0,
        purchases: 0,
        revenue: 0,
        spend: 0,
        costPerLead: null,
        costPerPurchase: null,
        roas: null,
        impressions: 0,
        clicks: 0,
        cpc: null,
        ctr: null,
        cpm: null,
        costPerConversation: null,
      },
      perAd: [],
      spendCurrency: null,
    })
    const messenger = await apiClient(tenant.token).get("/v1/ads/analytics/overview", { query: { ...DAYS, channel: "messenger" } })
    expect(messenger.json).toEqual(response.json)
    expect(await fake.requests({ kind: "http", host: GRAPH, since })).toEqual([])
  })

  it("rejects bad input with 422", async () => {
    const tenant = await createTenant({ label: "analytics-422" })
    const api = apiClient(tenant.token)
    expectValidationIssue(await api.get("/v1/ads/analytics/overview", { query: { ...DAYS, from: "2026/10/01" } }), ["from"])
    expectValidationIssue(await api.get("/v1/ads/analytics/overview", { query: { to: DAYS.to } }), ["from"])
    expectValidationIssue(await api.get("/v1/ads/analytics/overview", { query: { ...DAYS, adAccountId: "123" } }), [
      "adAccountId",
    ])
    expectValidationIssue(await api.get("/v1/ads/analytics/overview", { query: { ...DAYS, channel: "facebook" } }), [
      "channel",
    ])
    expectValidationIssue(
      await api.get("/v1/ads/analytics/overview", { query: { ...DAYS, channel: "messenger", allChannels: true } }),
      ["allChannels"],
    )
  })
})

describe("ads.getAnalyticsTimeseries (GET /v1/ads/analytics/timeseries)", () => {
  it("returns one row per day with funnel counts and daily Meta spend", async () => {
    const setup = await messengerAdsSetup({ label: "analytics-ts" })
    await seedFunnel(setup.tenant.workspaceId, setup)
    const adAccountId = uniqueAdAccountId()
    const since = await fake.cursor()
    await graphScript("GET", "/me/adaccounts", { status: 200, json: { data: [{ id: adAccountId }] } })
    await graphScript("GET", `/${adAccountId}/insights`, {
      status: 200,
      json: {
        data: [
          { ad_id: "120001", account_currency: "USD", spend: "7.5", impressions: "10", clicks: "1", date_start: "2026-10-02" },
          { ad_id: "120001", account_currency: "USD", spend: "2.5", impressions: "10", clicks: "1", date_start: "2026-10-03" },
          { ad_id: "120001", account_currency: "USD", spend: "1", impressions: "1", clicks: "0", date_start: "2026-09-30" },
        ],
      },
    })
    const response = await setup.api.get("/v1/ads/analytics/timeseries", { query: { ...DAYS, channel: "messenger" } })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      data: [
        { date: "2026-10-01", conversations: 0, leads: 0, purchases: 0, spend: null },
        { date: "2026-10-02", conversations: 1, leads: 1, purchases: 0, spend: 7.5 },
        { date: "2026-10-03", conversations: 1, leads: 0, purchases: 1, spend: 2.5 },
        { date: "2026-10-04", conversations: 0, leads: 0, purchases: 0, spend: null },
        { date: "2026-10-05", conversations: 0, leads: 0, purchases: 0, spend: null },
        { date: "2026-10-06", conversations: 0, leads: 0, purchases: 0, spend: null },
        { date: "2026-10-07", conversations: 0, leads: 0, purchases: 0, spend: null },
      ],
    })
    const calls = await graphCalls(setup.accessToken, since)
    expect(calls.map((call) => call.path)).toEqual(["/v23.0/me/adaccounts", `/v23.0/${adAccountId}/insights`])
    expect(calls[1]?.query).toEqual({
      level: "ad",
      fields: "ad_id,ad_name,account_currency,spend,impressions,clicks,date_start",
      time_range: JSON.stringify({ since: "2026-10-01", until: "2026-10-07" }),
      limit: "499",
      access_token: setup.accessToken,
      time_increment: "1",
    })
  })

  it("rejects bad dates with 422", async () => {
    const tenant = await createTenant({ label: "analytics-ts-422" })
    expectValidationIssue(
      await apiClient(tenant.token).get("/v1/ads/analytics/timeseries", { query: { from: "2026-10-01", to: "Oct 7" } }),
      ["to"],
    )
  })
})

describe("ads.listChannelAdAccounts (GET /v1/ads/{channel}/ad-accounts)", () => {
  it("unions the integrations' connections with the workspace-wide Facebook Ads connection", async () => {
    const setup = await messengerAdsSetup({ label: "channel-ad-accounts" })
    const ext = await adsFixtures()
    const workspaceToken = uniqueAccessToken("fbads")
    await ext.connectWorkspaceFacebookAds({ workspaceId: setup.tenant.workspaceId, accessToken: workspaceToken })
    const [a, b, c] = [uniqueAdAccountId(), uniqueAdAccountId(), uniqueAdAccountId()]
    const since = await fake.cursor()

    // 1) One integration: only its own connection's accounts.
    await graphScript("GET", "/me/adaccounts", { status: 200, json: { data: [{ id: a, name: "A" }, { id: b, name: "B" }] } }, { times: 1 })
    const scoped = await setup.api.get("/v1/ads/messenger/ad-accounts", { query: { integrationId: setup.integrationId } })
    expect(scoped.status, scoped.text).toBe(200)
    expect(scoped.json).toEqual({ data: [{ id: a, name: "A" }, { id: b, name: "B" }] })
    expect((await graphCalls(setup.accessToken, since)).map((call) => call.path)).toEqual(["/v23.0/me/adaccounts"])

    // 2) The channel: cached integration accounts + workspace-wide ones, deduped by id.
    await graphScript("GET", "/me/adaccounts", { status: 200, json: { data: [{ id: b, name: "B (workspace)" }, { id: c }] } }, { times: 1 })
    const union = await setup.api.get("/v1/ads/messenger/ad-accounts")
    expect(union.status, union.text).toBe(200)
    expect(union.json).toEqual({ data: [{ id: a, name: "A" }, { id: b, name: "B" }, { id: c }] })
    const workspaceCalls = await graphCalls(workspaceToken, since)
    expect(workspaceCalls.map((call) => ({ path: call.path, query: call.query }))).toEqual([
      { path: "/v23.0/me/adaccounts", query: { fields: "id,name", limit: "499", access_token: workspaceToken } },
    ])
    expect(await graphCalls(setup.accessToken, since)).toHaveLength(1)

    // 3) An integration without a connection, and another channel: empty.
    const unknown = await setup.api.get("/v1/ads/messenger/ad-accounts", { query: { integrationId: "1000000000000000001" } })
    expect(unknown.json).toEqual({ data: [] })
    const instagram = await setup.api.get("/v1/ads/instagram/ad-accounts")
    expect(instagram.json).toEqual({ data: [{ id: b, name: "B (workspace)" }, { id: c }] })
    expect(await fake.requests({ kind: "http", host: GRAPH, since })).toHaveLength(2)
  })

  it("ignores an invalid connection and rejects a bad channel with 422", async () => {
    const setup = await messengerAdsSetup({ label: "channel-ad-accounts-fail", status: "invalid" })
    const response = await setup.api.get("/v1/ads/messenger/ad-accounts")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ data: [] })
    expectValidationIssue(await setup.api.get("/v1/ads/facebook/ad-accounts"), ["channel"])
    expectValidationIssue(await setup.api.get("/v1/ads/messenger/ad-accounts", { query: { integrationId: "x" } }), [
      "integrationId",
    ])
  })
})

describe("ads.listCustomAudiences (GET /v1/ads/custom-audiences)", () => {
  it("lists the ad account's custom audiences through the workspace connection", async () => {
    const tenant = await createTenant({ label: "custom-audiences" })
    const ext = await adsFixtures()
    const accessToken = uniqueAccessToken("fbads")
    await ext.connectWorkspaceFacebookAds({ workspaceId: tenant.workspaceId, accessToken })
    const adAccountId = uniqueAdAccountId()
    const since = await fake.cursor()
    await graphScript("GET", `/${adAccountId}/customaudiences`, {
      status: 200,
      json: { data: [{ id: "6001", name: "Buyers", subtype: "CUSTOM" }, { id: "6002" }] },
    })
    const api = apiClient(tenant.token)
    const response = await api.get("/v1/ads/custom-audiences", { query: { adAccountId } })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ data: [{ id: "6001", name: "Buyers", subtype: "CUSTOM" }, { id: "6002" }] })
    const calls = await graphCalls(accessToken, since)
    expect(calls.map((call) => ({ path: call.path, query: call.query }))).toEqual([
      {
        path: `/v23.0/${adAccountId}/customaudiences`,
        query: { fields: "id,name,subtype", limit: "499", access_token: accessToken },
      },
    ])
    // Cached for 60 s.
    await api.get("/v1/ads/custom-audiences", { query: { adAccountId } })
    expect(await graphCalls(accessToken, since)).toHaveLength(1)
    expectValidationIssue(await api.get("/v1/ads/custom-audiences"), ["adAccountId"])
  })

  it("fails with 500 without a workspace Facebook Ads connection", async () => {
    const tenant = await createTenant({ label: "custom-audiences-none" })
    const response = await apiClient(tenant.token).get("/v1/ads/custom-audiences", { query: { adAccountId: "act_1" } })
    expectError(response, 500, "INTERNAL_SERVER_ERROR")
  })
})

describe("ads.startRetargetAudienceSync (POST /v1/ads/retarget-audiences)", () => {
  const sha256 = (value: string) => createHash("sha256").update(value).digest("hex")

  it("creates the audience, answers 202 and the worker uploads the segment's hashed contacts", async () => {
    const setup = await messengerAdsSetup({ label: "retarget", connect: false })
    const ext = await adsFixtures()
    const accessToken = uniqueAccessToken("fbads")
    await ext.connectWorkspaceFacebookAds({ workspaceId: setup.tenant.workspaceId, accessToken })
    const email = `Lead-${uniqueId()}@Parity.test`
    const contact = await ext.createContactInbox({
      workspaceId: setup.tenant.workspaceId,
      inboxId: setup.inboxId,
      channel: "messenger",
      sourceId: `psid-${uniqueId()}`,
      email,
      firstInteractionAt: "2026-10-02T09:00:00.000Z",
      referral: { adId: "120009", source: "ADS" },
    })
    await ext.createConversionEvent({
      workspaceId: setup.tenant.workspaceId,
      channel: "messenger",
      integrationId: setup.integrationId,
      eventType: "lead",
      occurredAt: "2026-10-02T10:00:00.000Z",
      sourceEventId: `lead-${uniqueId()}`,
      adId: "120009",
      contactInboxId: contact.contactInboxId,
    })
    const adAccountId = uniqueAdAccountId()
    const audienceId = uniqueDigits()
    const since = await fake.cursor()
    await graphScript("POST", `/${adAccountId}/customaudiences`, { status: 200, json: { id: audienceId } })
    await graphScript("POST", `/${audienceId}/users`, {
      status: 200,
      json: { audience_id: audienceId, num_received: 1, num_invalid_entries: 0 },
    })

    const response = await setup.api.post("/v1/ads/retarget-audiences", {
      segment: "leads",
      channel: "messenger",
      integrationMessengerId: setup.integrationId,
      since: "2026-10-01T00:00:00.000Z",
      until: "2026-10-07T23:59:59.999Z",
      adAccountId,
      audienceName: "Parity leads",
    })
    expect(response.status, response.text).toBe(202)
    expect(response.json).toEqual({ customAudienceId: audienceId, enqueued: true })

    const create = (await graphCalls(accessToken, since)).find((call) => call.path.endsWith("/customaudiences"))
    expect(create).toMatchObject({ method: "POST", path: `/v23.0/${adAccountId}/customaudiences`, query: { access_token: accessToken } })
    expect(jsonBody(create)).toEqual({ name: "Parity leads", subtype: "CUSTOM", customer_file_source: "USER_PROVIDED_ONLY" })

    const [users] = await fake.waitFor(
      { kind: "http", host: GRAPH, method: "POST", pathPrefix: `/v23.0/${audienceId}/users`, since },
      { timeoutMs: 20_000 },
    )
    expect(users?.query).toEqual({ access_token: accessToken })
    expect(jsonBody(users)).toEqual({
      payload: { schema: ["PHONE", "EMAIL"], data: [["", sha256(email.toLowerCase())]] },
    })
  })

  it("syncs into an existing audience without creating one", async () => {
    const tenant = await createTenant({ label: "retarget-existing" })
    const ext = await adsFixtures()
    const accessToken = uniqueAccessToken("fbads")
    await ext.connectWorkspaceFacebookAds({ workspaceId: tenant.workspaceId, accessToken })
    const audienceId = uniqueDigits()
    const since = await fake.cursor()
    const response = await apiClient(tenant.token).post("/v1/ads/retarget-audiences", {
      segment: "purchases",
      since: "2026-10-01T00:00:00Z",
      until: "2026-10-02T00:00:00Z",
      adAccountId: "act_1",
      customAudienceId: audienceId,
    })
    expect(response.status, response.text).toBe(202)
    expect(response.json).toEqual({ customAudienceId: audienceId, enqueued: true })
    // No contacts in the segment: the worker sends nothing.
    expect(await graphCalls(accessToken, since)).toEqual([])
  })

  it("rejects bad input with 422 and fails with 500 without a Facebook Ads connection", async () => {
    const tenant = await createTenant({ label: "retarget-422" })
    const api = apiClient(tenant.token)
    const body = { segment: "leads", since: "2026-10-01T00:00:00Z", until: "2026-10-02T00:00:00Z", adAccountId: "act_1" }
    expectValidationIssue(await api.post("/v1/ads/retarget-audiences", body), ["audienceName"])
    expectValidationIssue(await api.post("/v1/ads/retarget-audiences", { ...body, segment: "visits", audienceName: "x" }), [
      "segment",
    ])
    expectValidationIssue(await api.post("/v1/ads/retarget-audiences", { ...body, adAccountId: "", audienceName: "x" }), [
      "adAccountId",
    ])
    expectError(await api.post("/v1/ads/retarget-audiences", { ...body, audienceName: "x" }), 500, "INTERNAL_SERVER_ERROR")
  })
})
