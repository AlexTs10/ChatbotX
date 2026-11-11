// facebookMarketingMessages.campaigns.{stop,resume,send}: the delivery state
// machine (packages/business/src/facebook-marketing-message-delivery/service.ts).
// Campaigns are fixture rows in each start state. Campaigns that the worker
// may pick up after a transition use a Page that is not connected, so the
// worker pauses them (`pageNotConnected`) without any Graph call.

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { fake } from "../../src/fake.ts"
import {
  type Campaign,
  CAMPAIGN_KEYS,
  CONNECT_FIRST,
  getCampaign,
  graphCalls,
  graphError,
  insertCampaign,
  mmTenant,
  msFromNow,
  NOT_FOUND_MESSAGE,
} from "./helpers-mm.ts"
import { expectError, GRAPH, graphPath, keysOf, script } from "./helpers.ts"

const CAMPAIGNS = "/v1/marketing-messages/campaigns"
const INVALID_STATE = "This action is not available in the campaign's current state."
const ENDED = "The Meta message campaign has ended, so it cannot be sent or resumed."

describe("facebookMarketingMessages.campaigns.stop", () => {
  it("stops a scheduled, paused, sending or waiting campaign and clears its reason", async () => {
    const mm = await mmTenant("p108-mm-stop", { scriptDebugToken: false })
    const ws = mm.tenant.workspaceId
    const since = await fake.cursor()
    for (const status of ["scheduled", "paused", "waitingNextDay"] as const) {
      const row = await insertCampaign(ws, {
        status,
        statusReason: status === "paused" ? "quotaReached" : null,
        // Far in the future: the worker never starts these.
        startsAt: new Date(Date.now() + 86_400_000),
      })
      const response = await mm.api.post<Campaign>(`${CAMPAIGNS}/${row.id}/stop`, {})
      expect(response.status, response.text).toBe(200)
      expect(keysOf(response.json)).toEqual(CAMPAIGN_KEYS)
      expect(response.json).toMatchObject({ id: row.id, status: "stopped", statusReason: null, statusErrorCode: null })
    }
    // Stop never touches Meta.
    expect(await fake.requests({ kind: "http", host: GRAPH, since })).toHaveLength(0)
  })

  it("refuses a campaign that is not running (400 mmInvalidState); 404 for an unknown one", async () => {
    const mm = await mmTenant("p108-mm-stop-bad", { grant: "none", scriptDebugToken: false })
    for (const status of ["notSent", "stopped", "completed", "failed"] as const) {
      const row = await insertCampaign(mm.tenant.workspaceId, { status })
      expectError(await mm.api.post(`${CAMPAIGNS}/${row.id}/stop`, {}), 400, "mmInvalidState", {
        defined: false,
        message: INVALID_STATE,
      })
    }
    expectError(await mm.api.post(`${CAMPAIGNS}/123456789012345678/stop`, {}), 404, "mmNotFound", {
      message: NOT_FOUND_MESSAGE,
    })
  })
})

describe("facebookMarketingMessages.campaigns.resume", () => {
  it("resumes by snapshot state: no tokens yet -> scheduled now; lifetime -> sending; daily -> waitingNextDay", async () => {
    const mm = await mmTenant("p108-mm-resume", { scriptDebugToken: false })
    const ws = mm.tenant.workspaceId

    const noTokens = await insertCampaign(ws, { status: "stopped", tokenCount: null })
    const scheduled = await mm.api.post<Campaign>(`${CAMPAIGNS}/${noTokens.id}/resume`, {})
    expect(scheduled.status, scheduled.text).toBe(200)
    expect(keysOf(scheduled.json)).toEqual(CAMPAIGN_KEYS)
    expect(scheduled.json).toMatchObject({ status: "scheduled", statusReason: null, nextDayAt: null })
    expect(Math.abs(msFromNow(scheduled.json.startsAt))).toBeLessThan(30_000)

    const daily = await insertCampaign(ws, { status: "paused", statusReason: "quotaReached", tokenCount: 40, budgetType: "daily" })
    const waiting = await mm.api.post<Campaign>(`${CAMPAIGNS}/${daily.id}/resume`, {})
    expect(waiting.status, waiting.text).toBe(200)
    expect(waiting.json).toMatchObject({ status: "waitingNextDay", statusReason: null, tokenCount: 40 })
    expect(Math.abs(msFromNow(waiting.json.nextDayAt))).toBeLessThan(30_000)

    const lifetime = await insertCampaign(ws, { status: "paused", statusReason: "grantInvalid", tokenCount: 3, budgetType: "lifetime" })
    const sending = await mm.api.post<Campaign>(`${CAMPAIGNS}/${lifetime.id}/resume`, {})
    expect(sending.status, sending.text).toBe(200)
    expect(sending.json).toMatchObject({ status: "sending", statusReason: null, tokenCount: 3 })

    // The worker takes the rescheduled campaign; its Page is not connected,
    // so it is paused again with that reason.
    await eventually(
      async () => {
        const row = await getCampaign(mm.api, noTokens.id)
        expect(row).toMatchObject({ status: "paused", statusReason: "pageNotConnected" })
      },
      { timeoutMs: 150_000, intervalMs: 2_000, description: "worker pauses the resumed campaign" },
    )
  }, 180_000)

  it("refuses a campaign that is not paused or stopped, or whose Meta campaign ended", async () => {
    const mm = await mmTenant("p108-mm-resume-bad", { scriptDebugToken: false })
    const ws = mm.tenant.workspaceId
    for (const status of ["notSent", "completed", "failed"] as const) {
      const row = await insertCampaign(ws, { status })
      expectError(await mm.api.post(`${CAMPAIGNS}/${row.id}/resume`, {}), 400, "mmInvalidState", { message: INVALID_STATE })
    }
    const ended = await insertCampaign(ws, { status: "stopped", metaEndsAt: new Date(Date.now() - 60_000) })
    expectError(await mm.api.post(`${CAMPAIGNS}/${ended.id}/resume`, {}), 400, "mmCampaignEnded", {
      defined: false,
      message: ENDED,
    })
    expectError(await mm.api.post(`${CAMPAIGNS}/123456789012345678/resume`, {}), 404, "mmNotFound")
  })

  it("is 400 mmGrantInvalid without an active grant", async () => {
    const mm = await mmTenant("p108-mm-resume-nogrant", { grant: "invalid", scriptDebugToken: false })
    const row = await insertCampaign(mm.tenant.workspaceId, { status: "stopped" })
    expectError(await mm.api.post(`${CAMPAIGNS}/${row.id}/resume`, {}), 400, "mmGrantInvalid", { message: CONNECT_FIRST })
    expect((await getCampaign(mm.api, row.id)).status).toBe("stopped")
  })
})

const scriptSchedule = (campaignId: string, respond: { status: number; json: unknown }) =>
  script({ match: { host: GRAPH, method: "GET", pathRegex: graphPath(campaignId) }, respond })

describe("facebookMarketingMessages.campaigns.send", () => {
  it("reads the Meta schedule and schedules a notSent campaign to start now", async () => {
    const mm = await mmTenant("p108-mm-send", { scriptDebugToken: false })
    const row = await insertCampaign(mm.tenant.workspaceId, { status: "notSent" })
    const endTime = new Date(Date.now() + 10 * 86_400_000).toISOString()
    await scriptSchedule(row.campaignId, { status: 200, json: { id: row.campaignId, adset: { id: "555", end_time: endTime } } })
    const since = await fake.cursor()

    const response = await mm.api.post<Campaign>(`${CAMPAIGNS}/${row.id}/send`, {})
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(CAMPAIGN_KEYS)
    expect(response.json).toMatchObject({ id: row.id, status: "scheduled", statusReason: null })
    expect(Math.abs(msFromNow(response.json.startsAt))).toBeLessThan(30_000)

    const reads = await graphCalls(since, row.campaignId, "GET")
    expect(reads).toHaveLength(1)
    expect(reads[0]?.path).toBe(`/v23.0/${row.campaignId}`)
    expect(reads[0]?.query).toEqual({ access_token: mm.token, fields: "adset{id,end_time}" })
    // The stored grant is enough: no /debug_token round trip.
    expect(await graphCalls(since, "debug_token")).toHaveLength(0)

    await eventually(
      async () => expect(await getCampaign(mm.api, row.id)).toMatchObject({ status: "paused", statusReason: "pageNotConnected" }),
      { timeoutMs: 150_000, intervalMs: 2_000, description: "worker pauses the started campaign" },
    )
  }, 180_000)

  it("still sends when the schedule read fails with a non-token error", async () => {
    const mm = await mmTenant("p108-mm-send-schedule-error", { scriptDebugToken: false })
    const row = await insertCampaign(mm.tenant.workspaceId, {
      status: "notSent",
      metaEndsAt: new Date(Date.now() + 86_400_000),
    })
    await scriptSchedule(row.campaignId, { status: 400, json: graphError(100, "Unsupported get request.") })
    const response = await mm.api.post<Campaign>(`${CAMPAIGNS}/${row.id}/send`, {})
    expect(response.status, response.text).toBe(200)
    expect(response.json.status).toBe("scheduled")
    // Stop it at once so the worker never starts it.
    expect((await mm.api.post<Campaign>(`${CAMPAIGNS}/${row.id}/stop`, {})).json.status).toBe("stopped")
  })

  it("refuses a campaign whose Meta campaign has ended", async () => {
    const mm = await mmTenant("p108-mm-send-ended", { scriptDebugToken: false })
    const row = await insertCampaign(mm.tenant.workspaceId, { status: "notSent" })
    await scriptSchedule(row.campaignId, {
      status: 200,
      json: { id: row.campaignId, adset: { id: "556", end_time: new Date(Date.now() - 3_600_000).toISOString() } },
    })
    expectError(await mm.api.post(`${CAMPAIGNS}/${row.id}/send`, {}), 400, "mmCampaignEnded", { message: ENDED })
    expect((await getCampaign(mm.api, row.id)).status).toBe("notSent")
  })

  it("marks the grant invalid when Graph answers 190", async () => {
    const mm = await mmTenant("p108-mm-send-190", { scriptDebugToken: false })
    const row = await insertCampaign(mm.tenant.workspaceId, { status: "notSent" })
    await scriptSchedule(row.campaignId, { status: 400, json: graphError(190, "Error validating access token") })
    expectError(await mm.api.post(`${CAMPAIGNS}/${row.id}/send`, {}), 400, "mmGrantInvalid", { message: CONNECT_FIRST })
    // The grant is now `invalid`: the next send refuses without asking Graph.
    const since = await fake.cursor()
    expectError(await mm.api.post(`${CAMPAIGNS}/${row.id}/send`, {}), 400, "mmGrantInvalid")
    expect(await fake.requests({ kind: "http", host: GRAPH, since })).toHaveLength(0)
    expect((await getCampaign(mm.api, row.id)).status).toBe("notSent")
  })

  it("refuses any status but notSent; 404 for an unknown campaign; 400 without a grant", async () => {
    const mm = await mmTenant("p108-mm-send-bad", { scriptDebugToken: false })
    for (const status of ["scheduled", "paused", "stopped", "completed"] as const) {
      const row = await insertCampaign(mm.tenant.workspaceId, { status, startsAt: new Date(Date.now() + 86_400_000) })
      expectError(await mm.api.post(`${CAMPAIGNS}/${row.id}/send`, {}), 400, "mmInvalidState", { message: INVALID_STATE })
    }
    expectError(await mm.api.post(`${CAMPAIGNS}/123456789012345678/send`, {}), 404, "mmNotFound")

    const noGrant = await mmTenant("p108-mm-send-nogrant", { grant: "none", scriptDebugToken: false })
    const row = await insertCampaign(noGrant.tenant.workspaceId, { status: "notSent" })
    expectError(await noGrant.api.post(`${CAMPAIGNS}/${row.id}/send`, {}), 400, "mmGrantInvalid")
  })
})
