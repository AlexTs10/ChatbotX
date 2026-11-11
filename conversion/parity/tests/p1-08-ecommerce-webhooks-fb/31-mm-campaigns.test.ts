// facebookMarketingMessages.campaigns.{create,list,get,update,delete}.
// Meta first, database second: a campaign row exists only once Graph
// returned the message campaign (ad) id. Rows in delivery states the API
// cannot reach (paused, completed, ...) are fixture rows.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import {
  type Campaign,
  CAMPAIGN_KEYS,
  CONNECT_FIRST,
  getCampaign,
  graphAdAccount,
  graphCalls,
  graphError,
  insertAudience,
  insertCampaign,
  jsonBody,
  mmTenant,
  msFromNow,
  NOT_FOUND_MESSAGE,
  scriptAdAccounts,
  textContent,
  uniqueAct,
} from "./helpers-mm.ts"
import { ext, expectError, expectInvalid, GRAPH, graphPath, keysOf, script, uniqueDigits, uniqueName } from "./helpers.ts"

const CAMPAIGNS = "/v1/marketing-messages/campaigns"

/** A grant + one ToS-accepted ad account + a scripted message_campaign create. */
const createSetup = async (label: string, options: { currency?: string; tos?: boolean } = {}) => {
  const mm = await mmTenant(label)
  const act = uniqueAct()
  const pageId = uniqueDigits()
  const adId = uniqueDigits()
  await scriptAdAccounts([graphAdAccount(act, { currency: options.currency ?? "USD", tos: options.tos })])
  await script({
    match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${act}/message_campaign`) },
    respond: { status: 200, json: { id: adId } },
  })
  return { ...mm, act, pageId, adId }
}

const createBody = (input: { act: string; pageId: string } & Record<string, unknown>) => {
  const { act, pageId, ...rest } = input
  return {
    name: uniqueName("MM campaign"),
    pageId,
    adAccountId: act,
    budgetType: "daily",
    budgetMajorUnits: 30.5,
    content: textContent("Hello from parity"),
    targetType: "optIn",
    ...rest,
  }
}

describe("facebookMarketingMessages.campaigns.create", () => {
  it("creates the Meta message campaign, then a scheduled campaign starting in one hour", async () => {
    const setup = await createSetup("p108-mm-create")
    const since = await fake.cursor()
    const body = createBody({ act: setup.act, pageId: setup.pageId })

    const response = await setup.api.post<Campaign>(CAMPAIGNS, body)
    expect(response.status, response.text).toBe(201)
    expect(keysOf(response.json)).toEqual(CAMPAIGN_KEYS)
    expect(response.json).toMatchObject({
      name: body.name,
      pageId: setup.pageId,
      adAccountId: setup.act,
      currency: "USD",
      budgetType: "daily",
      budgetMinorUnits: 3050,
      content: { templateType: "text", text: "Hello from parity", quickReplies: [] },
      targetType: "optIn",
      audienceId: null,
      campaignId: setup.adId,
      status: "scheduled",
      statusReason: null,
      statusErrorCode: null,
      statusErrorMessage: null,
      startedAt: null,
      firstSentAt: null,
      completedAt: null,
      nextDayAt: null,
      tokenCount: null,
    })
    expect(response.json.id).toMatch(/^\d{15,20}$/)
    expect(Math.abs(msFromNow(response.json.startsAt) - 3_600_000)).toBeLessThan(60_000)
    expect(Math.abs(msFromNow(response.json.createdAt))).toBeLessThan(60_000)

    const creates = await graphCalls(since, `${setup.act}/message_campaign`, "POST")
    expect(creates).toHaveLength(1)
    expect(creates[0]?.path).toBe(`/v23.0/${setup.act}/message_campaign`)
    expect(jsonBody(creates[0])).toEqual({
      access_token: setup.token,
      name: body.name,
      page_id: setup.pageId,
      daily_budget: 3050,
    })

    // The stored row is what list and get return.
    const list = await setup.api.get<{ campaigns: Campaign[] }>(CAMPAIGNS)
    expect(list.status).toBe(200)
    expect(list.json.campaigns).toEqual([response.json])
    const detail = await getCampaign(setup.api, response.json.id)
    expect(detail).toEqual({
      ...response.json,
      stats: { counts: { pending: 0, sent: 0, delivered: 0, failed: 0, unconfirmed: 0 }, errors: [] },
    })
  })

  it("uses the ad account currency offset (VND: 1) and a lifetime budget; budget strings are coerced", async () => {
    const setup = await createSetup("p108-mm-create-vnd", { currency: "VND" })
    const since = await fake.cursor()
    const response = await setup.api.post<Campaign>(
      CAMPAIGNS,
      createBody({ act: setup.act, pageId: setup.pageId, budgetType: "lifetime", budgetMajorUnits: "150000" }),
    )
    expect(response.status, response.text).toBe(201)
    expect(response.json).toMatchObject({ currency: "VND", budgetType: "lifetime", budgetMinorUnits: 150000 })
    const [create] = await graphCalls(since, `${setup.act}/message_campaign`, "POST")
    expect(jsonBody(create)).toMatchObject({ lifetime_budget: 150000 })
    expect(jsonBody(create)).not.toHaveProperty("daily_budget")
  })

  it("targets a ready custom audience of the same ad account and Page", async () => {
    const setup = await createSetup("p108-mm-create-audience")
    const audience = await insertAudience(setup.tenant.workspaceId, {
      adAccountId: setup.act,
      pageId: setup.pageId,
      readyAt: new Date(Date.now() - 60_000),
      rowsUploaded: 150,
    })
    const response = await setup.api.post<Campaign>(
      CAMPAIGNS,
      createBody({ act: setup.act, pageId: setup.pageId, targetType: "customAudience", audienceId: audience.id }),
    )
    expect(response.status, response.text).toBe(201)
    expect(response.json).toMatchObject({ targetType: "customAudience", audienceId: audience.id })
  })

  it("refuses an audience that is not ready or belongs to another Page or account, before any Meta write", async () => {
    const setup = await createSetup("p108-mm-create-audience-bad")
    const ws = setup.tenant.workspaceId
    const notReady = await insertAudience(ws, { adAccountId: setup.act, pageId: setup.pageId, readyAt: null })
    const otherPage = await insertAudience(ws, { adAccountId: setup.act, readyAt: new Date() })
    const otherAccount = await insertAudience(ws, { pageId: setup.pageId, readyAt: new Date() })
    const since = await fake.cursor()
    for (const audienceId of [notReady.id, otherPage.id, otherAccount.id, "123456789012345678"]) {
      const response = await setup.api.post(
        CAMPAIGNS,
        createBody({ act: setup.act, pageId: setup.pageId, targetType: "customAudience", audienceId }),
      )
      expectError(response, 400, "systemError", {
        message: "The selected custom audience is not ready or does not belong to this Page and ad account.",
      })
    }
    expect(await graphCalls(since, `${setup.act}/message_campaign`, "POST")).toHaveLength(0)
    expect((await setup.api.get(CAMPAIGNS)).json).toEqual({ campaigns: [] })
  })

  it("refuses an ad account the grant does not see, or without the custom-audience ToS", async () => {
    const setup = await createSetup("p108-mm-create-account", { tos: false })
    const since = await fake.cursor()
    expectError(await setup.api.post(CAMPAIGNS, createBody({ act: uniqueAct(), pageId: setup.pageId })), 400, "systemError", {
      message: "Please grant access to use the Marketing Messages feature.",
    })
    expectError(await setup.api.post(CAMPAIGNS, createBody({ act: setup.act, pageId: setup.pageId })), 400, "systemError", {
      message: `To use this feature, please accept the custom object terms at https://business.facebook.com/ads/manage/customaudiences/tos/?act=${setup.act.slice(4)}`,
    })
    expect(await graphCalls(since, `${setup.act}/message_campaign`, "POST")).toHaveLength(0)
  })

  it("is 400 mmGrantInvalid without a grant", async () => {
    const mm = await mmTenant("p108-mm-create-nogrant", { grant: "none", scriptDebugToken: false })
    const since = await fake.cursor()
    const response = await mm.api.post(CAMPAIGNS, createBody({ act: uniqueAct(), pageId: uniqueDigits() }))
    expectError(response, 400, "mmGrantInvalid", { defined: false, message: CONNECT_FIRST })
    expect(await fake.requests({ kind: "http", host: GRAPH, since })).toHaveLength(0)
  })

  it("passes a Meta rejection through as 400 BAD_REQUEST and stores nothing", async () => {
    const mm = await mmTenant("p108-mm-create-meta-error")
    const act = uniqueAct()
    await scriptAdAccounts([graphAdAccount(act)])
    await script({
      match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${act}/message_campaign`) },
      respond: { status: 400, json: graphError(100, "Invalid parameter", 1885650) },
    })
    const response = await mm.api.post(CAMPAIGNS, createBody({ act, pageId: uniqueDigits() }))
    expectError(response, 400, "BAD_REQUEST")
    expect((await mm.api.get(CAMPAIGNS)).json).toEqual({ campaigns: [] })
  })

  it("validates the body (422)", async () => {
    const mm = await mmTenant("p108-mm-create-invalid", { scriptDebugToken: false })
    const base = createBody({ act: uniqueAct(), pageId: uniqueDigits() })
    expectInvalid(await mm.api.post(CAMPAIGNS, { ...base, targetType: "customAudience" }), "audienceId")
    expectInvalid(await mm.api.post(CAMPAIGNS, { ...base, budgetMajorUnits: 0 }), "budgetMajorUnits")
    expectInvalid(await mm.api.post(CAMPAIGNS, { ...base, budgetType: "weekly" }), "budgetType")
    expectInvalid(await mm.api.post(CAMPAIGNS, { ...base, name: "x".repeat(121) }), "name")
    expectInvalid(await mm.api.post(CAMPAIGNS, { ...base, adAccountId: "  " }), "adAccountId")
    expectInvalid(await mm.api.post(CAMPAIGNS, { ...base, content: { templateType: "text", text: "", quickReplies: [] } }), "content.text")
    expectInvalid(await mm.api.post(CAMPAIGNS, { ...base, targetType: "everyone" }), "targetType")
  })
})

describe("facebookMarketingMessages.campaigns.list / get", () => {
  it("lists the workspace's campaigns newest first; get adds delivery counts and the error breakdown", async () => {
    const mm = await mmTenant("p108-mm-list", { scriptDebugToken: false })
    const { tenant: other } = await mmTenant("p108-mm-list-other", { grant: "none", scriptDebugToken: false })
    const ws = mm.tenant.workspaceId
    const older = await insertCampaign(ws, {
      name: "Older",
      status: "completed",
      createdAt: new Date(Date.now() - 120_000),
      tokenCount: 7,
      timezoneName: "UTC",
    })
    const newer = await insertCampaign(ws, {
      name: "Newer",
      status: "paused",
      statusReason: "quotaReached",
      budgetType: "lifetime",
      currency: "VND",
      currencyOffset: 1,
      budgetMinorUnits: 200000,
      createdAt: new Date(Date.now() - 60_000),
    })
    await insertCampaign(other.workspaceId, { name: "Foreign" })
    await (await ext()).insertMarketingMessageRecipients({
      workspaceId: ws,
      campaignRowId: older.id,
      rows: [
        { status: "pending" },
        { status: "sent" },
        { status: "delivered" },
        { status: "delivered" },
        { status: "unconfirmed" },
        { status: "failed", errorCode: 551, errorMessage: "This person isn't available right now." },
        { status: "failed", errorCode: 551, errorMessage: "This person isn't available right now." },
        { status: "failed", errorCode: null, errorMessage: "Network timeout" },
      ],
    })

    const list = await mm.api.get<{ campaigns: Campaign[] }>(CAMPAIGNS)
    expect(list.status, list.text).toBe(200)
    expect(list.json.campaigns.map((row) => row.name)).toEqual(["Newer", "Older"])
    expect(list.json.campaigns.every((row) => keysOf(row).join() === CAMPAIGN_KEYS.join())).toBe(true)
    expect(list.json.campaigns[0]).toMatchObject({
      id: newer.id,
      campaignId: newer.campaignId,
      pageId: newer.pageId,
      adAccountId: newer.adAccountId,
      status: "paused",
      statusReason: "quotaReached",
      budgetType: "lifetime",
      budgetMinorUnits: 200000,
      currency: "VND",
      startsAt: null,
      tokenCount: null,
    })

    const detail = await getCampaign(mm.api, older.id)
    expect(keysOf(detail)).toEqual([...CAMPAIGN_KEYS, "stats"].sort())
    expect(detail).toMatchObject({ id: older.id, name: "Older", status: "completed", tokenCount: 7 })
    expect(detail.stats).toEqual({
      counts: { pending: 1, sent: 3, delivered: 2, failed: 3, unconfirmed: 1 },
      errors: [
        { errorCode: 551, errorMessage: "This person isn't available right now.", count: 2 },
        { errorCode: null, errorMessage: "Network timeout", count: 1 },
      ],
    })
  })

  it("get is 404 mmNotFound for an unknown or foreign campaign", async () => {
    const mm = await mmTenant("p108-mm-get-404", { grant: "none", scriptDebugToken: false })
    const { tenant: other } = await mmTenant("p108-mm-get-404-other", { grant: "none", scriptDebugToken: false })
    const foreign = await insertCampaign(other.workspaceId)
    for (const id of ["123456789012345678", foreign.id]) {
      expectError(await mm.api.get(`${CAMPAIGNS}/${id}`), 404, "mmNotFound", { defined: false, message: NOT_FOUND_MESSAGE })
    }
  })

  it("list is empty for a new workspace", async () => {
    const mm = await mmTenant("p108-mm-list-empty", { grant: "none", scriptDebugToken: false })
    const response = await mm.api.get(CAMPAIGNS)
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ campaigns: [] })
  })
})

/** Scripts the three Graph calls of an edit: read the ad set, write its budget, rename the ad. */
const scriptUpdate = async (campaignId: string, options: { budgetStatus?: number } = {}) => {
  const adSetId = uniqueDigits()
  await script(
    {
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(campaignId) },
      respond: { status: 200, json: { id: campaignId, adset: { id: adSetId } } },
    },
    {
      match: { host: GRAPH, method: "POST", pathRegex: graphPath(adSetId) },
      respond:
        options.budgetStatus && options.budgetStatus >= 400
          ? { status: options.budgetStatus, json: graphError(100, "The budget is too low.", 1885272) }
          : { status: 200, json: { success: true } },
    },
    {
      match: { host: GRAPH, method: "POST", pathRegex: graphPath(campaignId) },
      respond: { status: 200, json: { success: true } },
    },
  )
  return adSetId
}

describe("facebookMarketingMessages.campaigns.update", () => {
  it("writes the budget on the ad set and the name on the ad, then the row; budget type and currency stay", async () => {
    const mm = await mmTenant("p108-mm-update")
    const row = await insertCampaign(mm.tenant.workspaceId, {
      status: "scheduled",
      startsAt: new Date(Date.now() + 3_600_000),
      budgetType: "lifetime",
      budgetMinorUnits: 5000,
    })
    const adSetId = await scriptUpdate(row.campaignId)
    const since = await fake.cursor()

    const response = await mm.api.put<Campaign>(`${CAMPAIGNS}/${row.id}`, {
      name: "Renamed campaign",
      budgetMajorUnits: 12.34,
      content: textContent("New text"),
      // Frozen fields are not part of the edit body; extra keys are dropped.
      budgetType: "daily",
      adAccountId: "act_1",
    })
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(CAMPAIGN_KEYS)
    expect(response.json).toMatchObject({
      id: row.id,
      name: "Renamed campaign",
      budgetType: "lifetime",
      budgetMinorUnits: 1234,
      currency: "USD",
      adAccountId: row.adAccountId,
      content: { templateType: "text", text: "New text", quickReplies: [] },
      status: "scheduled",
    })

    const calls = (await fake.requests({ kind: "http", host: GRAPH, since })).filter(
      (record) => record.path.endsWith(`/${row.campaignId}`) || record.path.endsWith(`/${adSetId}`),
    )
    expect(calls.map((record) => `${record.method} ${record.path}`)).toEqual([
      `GET /v23.0/${row.campaignId}`,
      `POST /v23.0/${adSetId}`,
      `POST /v23.0/${row.campaignId}`,
    ])
    expect(calls[0]?.query).toEqual({ access_token: mm.token, fields: "adset{id}" })
    expect(jsonBody(calls[1])).toEqual({ access_token: mm.token, lifetime_budget: 1234 })
    expect(jsonBody(calls[2])).toEqual({ access_token: mm.token, name: "Renamed campaign" })
    expect((await getCampaign(mm.api, row.id)).name).toBe("Renamed campaign")
  })

  it("locks the content once the campaign has started; name and budget stay editable", async () => {
    const mm = await mmTenant("p108-mm-update-locked")
    const content = textContent("Original")
    const row = await insertCampaign(mm.tenant.workspaceId, { status: "paused", statusReason: "quotaReached", content })
    const since = await fake.cursor()

    const locked = await mm.api.put(`${CAMPAIGNS}/${row.id}`, {
      name: "Edited",
      budgetMajorUnits: 20,
      content: textContent("Changed"),
    })
    expectError(locked, 400, "mmContentLocked", {
      defined: false,
      message: "The content cannot be changed after the first message has been sent.",
    })
    // Refused before the grant check and any Graph call.
    expect(await fake.requests({ kind: "http", host: GRAPH, since })).toHaveLength(0)

    await scriptUpdate(row.campaignId)
    const allowed = await mm.api.put<Campaign>(`${CAMPAIGNS}/${row.id}`, {
      name: "Edited",
      budgetMajorUnits: 20,
      content,
    })
    expect(allowed.status, allowed.text).toBe(200)
    expect(allowed.json).toMatchObject({ name: "Edited", budgetMinorUnits: 2000, status: "paused", content })
  })

  it("is 400 mmNotFound (not 404) for an unknown campaign", async () => {
    const mm = await mmTenant("p108-mm-update-404", { scriptDebugToken: false })
    const response = await mm.api.put(`${CAMPAIGNS}/123456789012345678`, {
      name: "X",
      budgetMajorUnits: 1,
      content: textContent(),
    })
    expectError(response, 400, "mmNotFound", { defined: false, message: NOT_FOUND_MESSAGE })
  })

  it("keeps the row when Meta rejects the budget, and never renames the ad", async () => {
    const mm = await mmTenant("p108-mm-update-meta-error")
    const row = await insertCampaign(mm.tenant.workspaceId, { name: "Kept name", status: "notSent" })
    await scriptUpdate(row.campaignId, { budgetStatus: 400 })
    const since = await fake.cursor()
    const response = await mm.api.put(`${CAMPAIGNS}/${row.id}`, {
      name: "Not saved",
      budgetMajorUnits: 0.01,
      content: textContent(),
    })
    expectError(response, 400, "BAD_REQUEST")
    expect(await graphCalls(since, row.campaignId, "POST")).toHaveLength(0)
    expect(await getCampaign(mm.api, row.id)).toMatchObject({ name: "Kept name", budgetMinorUnits: 1000 })
  })

  it("is 400 mmGrantInvalid without a grant (after the content check)", async () => {
    const mm = await mmTenant("p108-mm-update-nogrant", { grant: "none", scriptDebugToken: false })
    const row = await insertCampaign(mm.tenant.workspaceId)
    expectError(
      await mm.api.put(`${CAMPAIGNS}/${row.id}`, { name: "X", budgetMajorUnits: 1, content: textContent() }),
      400,
      "mmGrantInvalid",
      { message: CONNECT_FIRST },
    )
  })

  it("validates the body (422)", async () => {
    const mm = await mmTenant("p108-mm-update-invalid", { scriptDebugToken: false })
    const path = `${CAMPAIGNS}/123456789012345678`
    expectInvalid(await mm.api.put(path, { name: "X", budgetMajorUnits: 1 }), "content")
    expectInvalid(await mm.api.put(path, { name: "", budgetMajorUnits: 1, content: textContent() }), "name")
    expectInvalid(await mm.api.put(path, { name: "X", budgetMajorUnits: -1, content: textContent() }), "budgetMajorUnits")
  })
})

describe("facebookMarketingMessages.campaigns.delete", () => {
  it("deletes the Meta campaign first, then the row", async () => {
    const mm = await mmTenant("p108-mm-delete", { scriptDebugToken: false })
    const row = await insertCampaign(mm.tenant.workspaceId, { status: "completed" })
    await script({
      match: { host: GRAPH, method: "DELETE", pathRegex: graphPath(row.campaignId) },
      respond: { status: 200, json: { success: true } },
    })
    const since = await fake.cursor()
    const response = await mm.api.delete(`${CAMPAIGNS}/${row.id}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ id: row.id })
    const calls = await graphCalls(since, row.campaignId, "DELETE")
    expect(calls).toHaveLength(1)
    expect(calls[0]?.path).toBe(`/v23.0/${row.campaignId}`)
    expect(calls[0]?.query).toEqual({ access_token: mm.token })
    // Delete resolves the stored grant only: no /debug_token round trip.
    expect(await graphCalls(since, "debug_token")).toHaveLength(0)
    expectError(await mm.api.get(`${CAMPAIGNS}/${row.id}`), 404, "mmNotFound")
  })

  it("removes the row when Meta says the campaign no longer exists (100/33)", async () => {
    const mm = await mmTenant("p108-mm-delete-gone", { scriptDebugToken: false })
    const row = await insertCampaign(mm.tenant.workspaceId)
    await script({
      match: { host: GRAPH, method: "DELETE", pathRegex: graphPath(row.campaignId) },
      respond: { status: 400, json: graphError(100, "Unsupported delete request.", 33) },
    })
    const response = await mm.api.delete(`${CAMPAIGNS}/${row.id}`)
    expect(response.status, response.text).toBe(200)
    expectError(await mm.api.get(`${CAMPAIGNS}/${row.id}`), 404, "mmNotFound")
  })

  it("keeps the row when Meta refuses (400 mmMetaDeleteFailed)", async () => {
    const mm = await mmTenant("p108-mm-delete-refused", { scriptDebugToken: false })
    const row = await insertCampaign(mm.tenant.workspaceId)
    await script({
      match: { host: GRAPH, method: "DELETE", pathRegex: graphPath(row.campaignId) },
      respond: { status: 400, json: graphError(200, "Permissions error") },
    })
    const response = await mm.api.delete(`${CAMPAIGNS}/${row.id}`)
    const body = expectError(response, 400, "mmMetaDeleteFailed", { defined: false })
    expect(body.message).toMatch(/^Meta could not delete this campaign, so it was kept\. /)
    expect((await getCampaign(mm.api, row.id)).id).toBe(row.id)
  })

  it("answers 200 for an unknown id without calling Meta", async () => {
    const mm = await mmTenant("p108-mm-delete-unknown", { grant: "none", scriptDebugToken: false })
    const since = await fake.cursor()
    const response = await mm.api.delete(`${CAMPAIGNS}/123456789012345678`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ id: "123456789012345678" })
    expect(await fake.requests({ kind: "http", host: GRAPH, since })).toHaveLength(0)
  })

  it("is 400 mmGrantInvalid without an active grant and keeps the row", async () => {
    for (const grant of ["none", "invalid"] as const) {
      const mm = await mmTenant(`p108-mm-delete-${grant}`, { grant, scriptDebugToken: false })
      const row = await insertCampaign(mm.tenant.workspaceId)
      expectError(await mm.api.delete(`${CAMPAIGNS}/${row.id}`), 400, "mmGrantInvalid", { message: CONNECT_FIRST })
      expect((await getCampaign(mm.api, row.id)).id).toBe(row.id)
    }
  })
})
