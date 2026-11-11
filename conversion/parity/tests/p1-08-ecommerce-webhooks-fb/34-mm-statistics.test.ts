// facebookMarketingMessages.statistics.get: campaign rows from the database
// (search, sort, paging), Meta insights for exactly the current page.
// Note: the per-ad (`level=ad`) and aggregate (`level=account`) insights
// calls share one Graph path; the fake upstream matches on path only, so
// both get the same scripted answer. The answer's first row has no `ad_id`
// (the aggregate), the others are per ad.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import {
  CONNECT_FIRST,
  dayIn,
  graphAdAccount,
  graphCalls,
  graphError,
  insertCampaign,
  minusDays,
  mmTenant,
  scriptAdAccounts,
  uniqueAct,
} from "./helpers-mm.ts"
import { expectError, expectInvalid, GRAPH, graphPath, script } from "./helpers.ts"

const STATS = "/v1/marketing-messages/statistics"
const NULL_METRICS = {
  delivered: null,
  readRate: null,
  linkClicks: null,
  linkClickRate: null,
  costPerClick: null,
  costPerDelivery: null,
  spend: null,
}
const EFFECTIVE_STATUSES = [
  "ACTIVE",
  "PAUSED",
  "DELETED",
  "PENDING_REVIEW",
  "DISAPPROVED",
  "PREAPPROVED",
  "PENDING_BILLING_INFO",
  "CAMPAIGN_PAUSED",
  "ARCHIVED",
  "ADSET_PAUSED",
  "IN_PROCESS",
  "WITH_ISSUES",
]
const METRIC_FIELDS = [
  "marketing_messages_delivered",
  "marketing_messages_read_rate",
  "marketing_messages_link_btn_click",
  "marketing_messages_link_btn_click_rate",
  "marketing_messages_cost_per_link_btn_click",
  "marketing_messages_cost_per_delivered",
  "marketing_messages_spend",
]

/** Two campaigns on act1 (older), one on act2 (newest). */
const statsSetup = async (label: string, options: { grant?: "active" | "none" } = {}) => {
  const mm = await mmTenant(label, { grant: options.grant ?? "active", scriptDebugToken: options.grant !== "none" })
  const ws = mm.tenant.workspaceId
  const act1 = uniqueAct()
  const act2 = uniqueAct()
  const alpha = await insertCampaign(ws, {
    adAccountId: act1,
    name: "Alpha promo",
    status: "completed",
    currency: "VND",
    currencyOffset: 1,
    timezoneName: "Asia/Ho_Chi_Minh",
    createdAt: new Date(Date.now() - 180_000),
  })
  const beta = await insertCampaign(ws, {
    adAccountId: act1,
    name: "Beta sale",
    status: "completed",
    currency: "VND",
    currencyOffset: 1,
    timezoneName: "Asia/Ho_Chi_Minh",
    createdAt: new Date(Date.now() - 120_000),
  })
  const gamma = await insertCampaign(ws, {
    adAccountId: act2,
    name: "Gamma",
    status: "completed",
    currency: "EUR",
    timezoneName: "Europe/Berlin",
    createdAt: new Date(Date.now() - 60_000),
  })
  return { ...mm, act1, act2, alpha, beta, gamma }
}

const scriptInsights = (act: string, data: unknown[], status = 200) =>
  script({
    match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${act}/insights`) },
    respond: status >= 400 ? { status, json: graphError(100, "Invalid parameter") } : { status: 200, json: { data } },
  })

const alphaInsight = (adId: string) => ({
  ad_id: adId,
  account_currency: "USD",
  marketing_messages_delivered: "120",
  marketing_messages_read_rate: "24.766355",
  marketing_messages_link_btn_click: 5,
  marketing_messages_link_btn_click_rate: "4.1",
  marketing_messages_cost_per_link_btn_click: "0.2",
  marketing_messages_cost_per_delivered: "0.01",
  marketing_messages_spend: "1.25",
})

const totalRow = {
  account_currency: "USD",
  marketing_messages_delivered: "200",
  marketing_messages_read_rate: "30",
  marketing_messages_link_btn_click: "9",
  marketing_messages_link_btn_click_rate: "",
  marketing_messages_cost_per_link_btn_click: "0.3",
  marketing_messages_cost_per_delivered: "0.02",
  marketing_messages_spend: { unexpected: "shape" },
}

describe("facebookMarketingMessages.statistics.get", () => {
  it("returns the page's campaigns with Meta's per-ad and aggregate insights", async () => {
    const setup = await statsSetup("p108-mm-stats")
    await scriptAdAccounts([
      graphAdAccount(setup.act1, { name: "Account One", timezone: "America/New_York" }),
      graphAdAccount(setup.act2, { name: "Account Two" }),
    ])
    await scriptInsights(setup.act1, [totalRow, alphaInsight(setup.alpha.campaignId)])
    const since = await fake.cursor()

    const query = `?adAccountId=${setup.act1}&since=2026-10-01&until=2026-10-31`
    const response = await setup.api.get(`${STATS}${query}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      adAccountId: setup.act1,
      adAccounts: [
        { id: setup.act2, name: "Account Two" },
        { id: setup.act1, name: "Account One" },
      ],
      currency: "USD",
      timezone: "America/New_York",
      since: "2026-10-01",
      until: "2026-10-31",
      rows: [
        { id: setup.beta.id, campaignId: setup.beta.campaignId, name: "Beta sale", pageId: setup.beta.pageId, ...NULL_METRICS },
        {
          id: setup.alpha.id,
          campaignId: setup.alpha.campaignId,
          name: "Alpha promo",
          pageId: setup.alpha.pageId,
          delivered: 120,
          readRate: 24.766355,
          linkClicks: 5,
          linkClickRate: 4.1,
          costPerClick: 0.2,
          costPerDelivery: 0.01,
          spend: 1.25,
        },
      ],
      aggregate: {
        delivered: 200,
        readRate: 30,
        linkClicks: 9,
        linkClickRate: null,
        costPerClick: 0.3,
        costPerDelivery: 0.02,
        spend: null,
      },
      aggregateUnavailable: false,
      pagination: { page: 1, perPage: 50, total: 2, pageCount: 1 },
    })

    const accounts = await graphCalls(since, "me/adaccounts", "GET")
    expect(accounts.filter((record) => record.query.access_token === setup.token)).toHaveLength(1)
    const insights = await graphCalls(since, `${setup.act1}/insights`, "GET")
    expect(insights.map((record) => record.query.level).sort()).toEqual(["account", "ad"])
    for (const record of insights) {
      const level = record.query.level
      expect(record.path).toBe(`/v23.0/${setup.act1}/insights`)
      expect(record.query).toMatchObject({ limit: "499", access_token: setup.token })
      expect(record.query.fields).toBe(
        [...(level === "ad" ? ["ad_id"] : []), "account_currency", ...METRIC_FIELDS].join(","),
      )
      expect(JSON.parse(String(record.query.time_range))).toEqual({ since: "2026-10-01", until: "2026-10-31" })
      expect(JSON.parse(String(record.query.filtering))).toEqual([
        { field: "ad.id", operator: "IN", value: [setup.beta.campaignId, setup.alpha.campaignId] },
        { field: "ad.effective_status", operator: "IN", value: EFFECTIVE_STATUSES },
      ])
    }

    // Insights and ad accounts are cached per workspace for 10 minutes.
    const again = await fake.cursor()
    const second = await setup.api.get(`${STATS}${query}`)
    expect(second.status).toBe(200)
    expect(second.json).toEqual(response.json)
    expect(await graphCalls(again, `${setup.act1}/insights`)).toHaveLength(0)
    expect(await graphCalls(again, "me/adaccounts")).toHaveLength(0)
  })

  it("defaults to the newest campaign's ad account and the last 30 days in its timezone", async () => {
    const setup = await statsSetup("p108-mm-stats-default")
    // No timezone from Meta: the stored one is used. No insight rows: null metrics, store currency.
    await scriptAdAccounts([graphAdAccount(setup.act1), graphAdAccount(setup.act2, { name: "Account Two" })])
    await scriptInsights(setup.act2, [])
    const response = await setup.api.get(STATS)
    expect(response.status, response.text).toBe(200)
    const until = dayIn("Europe/Berlin")
    expect(response.json).toEqual({
      adAccountId: setup.act2,
      adAccounts: [
        { id: setup.act2, name: "Account Two" },
        { id: setup.act1, name: `Account ${setup.act1}` },
      ],
      currency: "EUR",
      timezone: "Europe/Berlin",
      since: minusDays(until, 29),
      until,
      rows: [{ id: setup.gamma.id, campaignId: setup.gamma.campaignId, name: "Gamma", pageId: setup.gamma.pageId, ...NULL_METRICS }],
      aggregate: null,
      aggregateUnavailable: false,
      pagination: { page: 1, perPage: 50, total: 1, pageCount: 1 },
    })
  })

  it("searches by name part or exact campaign id, sorts and pages", async () => {
    const setup = await statsSetup("p108-mm-stats-search")
    await scriptAdAccounts([graphAdAccount(setup.act1), graphAdAccount(setup.act2)])
    await scriptInsights(setup.act1, [])
    const names = async (query: string) => {
      const response = await setup.api.get<{ rows: Array<{ name: string }>; pagination: unknown }>(
        `${STATS}?adAccountId=${setup.act1}&since=2026-10-01&until=2026-10-31&${query}`,
      )
      expect(response.status, response.text).toBe(200)
      return { names: response.json.rows.map((row) => row.name), pagination: response.json.pagination }
    }
    expect((await names("search=ALPHA")).names).toEqual(["Alpha promo"])
    expect((await names(`search=${setup.beta.campaignId}`)).names).toEqual(["Beta sale"])
    expect((await names("search=nothing")).names).toEqual([])
    expect((await names("sort=name")).names).toEqual(["Alpha promo", "Beta sale"])
    expect((await names("sort=name&order=desc")).names).toEqual(["Beta sale", "Alpha promo"])
    expect(await names("perPage=1&page=2")).toEqual({
      names: ["Alpha promo"],
      pagination: { page: 2, perPage: 1, total: 2, pageCount: 2 },
    })
  })

  it("degrades to stored timezone and unnamed accounts when the ad account read fails; insights errors are 400", async () => {
    const setup = await statsSetup("p108-mm-stats-degraded")
    await scriptAdAccounts([], { status: 400, times: 1 })
    await scriptInsights(setup.act1, [alphaInsight(setup.alpha.campaignId)])
    const response = await setup.api.get<Record<string, unknown>>(
      `${STATS}?adAccountId=${setup.act1}&since=2026-10-01&until=2026-10-31&search=Alpha`,
    )
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({
      adAccounts: [
        { id: setup.act2, name: null },
        { id: setup.act1, name: null },
      ],
      timezone: "Asia/Ho_Chi_Minh",
      currency: "USD",
      // The aggregate call got the same single per-ad row.
      aggregate: { delivered: 120, spend: 1.25 },
    })

    await scriptInsights(setup.act1, [], 400)
    const failed = await setup.api.get(`${STATS}?adAccountId=${setup.act1}&since=2026-09-01&until=2026-09-30`)
    expectError(failed, 400, "BAD_REQUEST")
  })

  it("answers the empty state without campaigns, with no grant and no Graph call", async () => {
    const mm = await mmTenant("p108-mm-stats-empty", { grant: "none", scriptDebugToken: false })
    const since = await fake.cursor()
    const response = await mm.api.get(STATS)
    expect(response.status, response.text).toBe(200)
    const until = dayIn("UTC")
    expect(response.json).toEqual({
      adAccountId: null,
      adAccounts: [],
      currency: null,
      timezone: "UTC",
      since: minusDays(until, 29),
      until,
      rows: [],
      aggregate: null,
      aggregateUnavailable: false,
      pagination: { page: 1, perPage: 50, total: 0, pageCount: 0 },
    })
    const explicit = await mm.api.get(`${STATS}?since=2026-01-01&until=2026-01-31&page=3&perPage=5`)
    expect(explicit.json).toMatchObject({ since: "2026-01-01", until: "2026-01-31", pagination: { page: 3, perPage: 5, total: 0, pageCount: 0 } })
    expect(await fake.requests({ kind: "http", host: GRAPH, since })).toHaveLength(0)
  })

  it("rejects a bad range or an ad account without campaigns (400) and needs a grant once there are campaigns", async () => {
    const setup = await statsSetup("p108-mm-stats-errors", { grant: "none" })
    expectError(await setup.api.get(`${STATS}?since=2026-10-31&until=2026-10-01`), 400, "mmStatisticsInvalidRange", {
      defined: false,
      message: "Choose a start date on or before the end date, within the last 37 months.",
    })
    expectError(await setup.api.get(`${STATS}?since=2023-01-01&until=2026-10-31`), 400, "mmStatisticsInvalidRange")
    expectError(await setup.api.get(`${STATS}?adAccountId=${uniqueAct()}`), 400, "mmStatisticsUnknownAdAccount", {
      defined: false,
      message: "This workspace has no Marketing Messages campaigns on that ad account.",
    })
    expectError(await setup.api.get(STATS), 400, "mmGrantInvalid", { message: CONNECT_FIRST })
  })

  it("validates the query (422)", async () => {
    const mm = await mmTenant("p108-mm-stats-422", { grant: "none", scriptDebugToken: false })
    expectInvalid(await mm.api.get(`${STATS}?adAccountId=123`), "adAccountId")
    expectInvalid(await mm.api.get(`${STATS}?since=2026/10/01`), "since")
    expectInvalid(await mm.api.get(`${STATS}?until=yesterday`), "until")
    expectInvalid(await mm.api.get(`${STATS}?sort=createdAt`), "sort")
    expectInvalid(await mm.api.get(`${STATS}?order=up`), "order")
    expectInvalid(await mm.api.get(`${STATS}?perPage=0`), "perPage")
    expectInvalid(await mm.api.get(`${STATS}?search=${"x".repeat(201)}`), "search")
  })
})
