// facebookMarketingMessages.audiences.{list,liveStatus,create,uploadMore,delete}.
// Rows are hashed server-side (sha256 of the normalized value) and uploaded
// to Meta in one session; `create` reports `ok`/`partial`/`invalid`
// outcomes instead of throwing for anything it can classify.

import { createHash } from "node:crypto"
import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import {
  AUDIENCE_KEYS,
  CONNECT_FIRST,
  graphAdAccount,
  graphCalls,
  graphError,
  insertAudience,
  insertCampaign,
  jsonBody,
  mmTenant,
  msFromNow,
  scriptAdAccounts,
  uniqueAct,
} from "./helpers-mm.ts"
import { expectError, expectInvalid, GRAPH, graphPath, keysOf, messengerPage, script, uniqueDigits } from "./helpers.ts"

const AUDIENCES = "/v1/marketing-messages/audiences"
const sha = (value: string) => createHash("sha256").update(value, "utf8").digest("hex")

type Audience = {
  id: string
  customAudienceId: string
  adAccountId: string
  pageId: string
  name: string
  description: string | null
  rowsUploaded: number
  readyAt: string | null
  createdAt: string
}

const listAudiences = async (api: Awaited<ReturnType<typeof mmTenant>>["api"], query = "") => {
  const response = await api.get<{ audiences: Audience[] }>(`${AUDIENCES}${query}`)
  expect(response.status, response.text).toBe(200)
  return response.json.audiences
}

describe("facebookMarketingMessages.audiences.list", () => {
  it("lists the workspace's audiences newest first, filtered by ad account, Page and readiness", async () => {
    const mm = await mmTenant("p108-mm-aud-list", { grant: "none", scriptDebugToken: false })
    const { tenant: other } = await mmTenant("p108-mm-aud-list-other", { grant: "none", scriptDebugToken: false })
    const ws = mm.tenant.workspaceId
    const act = uniqueAct()
    const pageId = uniqueDigits()
    const readyAt = new Date(Date.now() - 30_000)
    const a = await insertAudience(ws, {
      adAccountId: act,
      pageId,
      name: "Oldest",
      description: "First list",
      rowsUploaded: 120,
      readyAt,
      createdAt: new Date(Date.now() - 180_000),
    })
    const b = await insertAudience(ws, { adAccountId: act, name: "Middle", createdAt: new Date(Date.now() - 120_000) })
    const c = await insertAudience(ws, { pageId, name: "Newest", createdAt: new Date(Date.now() - 60_000) })
    await insertAudience(other.workspaceId, { adAccountId: act, pageId })

    const all = await listAudiences(mm.api)
    expect(all.map((row) => row.id)).toEqual([c.id, b.id, a.id])
    expect(all.every((row) => keysOf(row).join() === AUDIENCE_KEYS.join())).toBe(true)
    expect(all[2]).toEqual({
      id: a.id,
      customAudienceId: a.customAudienceId,
      adAccountId: act,
      pageId,
      name: "Oldest",
      description: "First list",
      rowsUploaded: 120,
      readyAt: readyAt.toISOString(),
      createdAt: all[2]?.createdAt,
    })
    expect(all[0]).toMatchObject({ description: null, rowsUploaded: 0, readyAt: null })

    expect((await listAudiences(mm.api, `?adAccountId=${act}`)).map((row) => row.id)).toEqual([b.id, a.id])
    expect((await listAudiences(mm.api, `?pageId=${pageId}`)).map((row) => row.id)).toEqual([c.id, a.id])
    expect((await listAudiences(mm.api, "?readyOnly=true")).map((row) => row.id)).toEqual([a.id])
    expect((await listAudiences(mm.api, `?adAccountId=${act}&pageId=${pageId}&readyOnly=false`)).map((row) => row.id)).toEqual([a.id])
  })

  it("validates readyOnly (422)", async () => {
    const mm = await mmTenant("p108-mm-aud-list-invalid", { grant: "none", scriptDebugToken: false })
    expectInvalid(await mm.api.get(`${AUDIENCES}?readyOnly=maybe`), "readyOnly")
  })
})

describe("facebookMarketingMessages.audiences.liveStatus", () => {
  it("combines Meta's audience counts with the subscription-token ready check, keyed by customAudienceId", async () => {
    const mm = await mmTenant("p108-mm-aud-live")
    const ws = mm.tenant.workspaceId
    const page = await messengerPage(ws, "Live page")
    const actA = uniqueAct()
    const actB = uniqueAct()
    const pending = await insertAudience(ws, { adAccountId: actA, pageId: page.pageId, readyAt: null })
    const ready = await insertAudience(ws, { adAccountId: actA, pageId: page.pageId, readyAt: new Date(Date.now() - 60_000) })
    const disconnected = await insertAudience(ws, { adAccountId: actA, pageId: uniqueDigits(), readyAt: null })
    const missing = await insertAudience(ws, { adAccountId: actB, pageId: page.pageId, readyAt: null })
    await script(
      {
        match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${actA}/customaudiences`) },
        respond: {
          status: 200,
          json: {
            data: [
              {
                id: pending.customAudienceId,
                name: "Pending",
                approximate_count_lower_bound: 1000,
                approximate_count_upper_bound: 1200,
                delivery_status: { code: 200, description: "This audience is ready" },
              },
              { id: ready.customAudienceId, approximate_count_lower_bound: 50, approximate_count_upper_bound: 60 },
              { id: disconnected.customAudienceId, delivery_status: { code: 300, description: "Too small" } },
            ],
          },
        },
      },
      {
        match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${actB}/customaudiences`) },
        respond: { status: 200, json: { data: [] } },
      },
      {
        match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${page.pageId}/notification_message_tokens`) },
        respond: { status: 200, json: { data: [{ custom_audience_ids: [pending.customAudienceId] }] } },
      },
    )
    const since = await fake.cursor()

    const response = await mm.api.get<{ statuses: Record<string, unknown> }>(`${AUDIENCES}/live`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      statuses: {
        [pending.customAudienceId]: {
          lower: 1000,
          upper: 1200,
          deliveryStatus: { code: 200, description: "This audience is ready" },
          ready: true,
          missingOnMeta: false,
          pageDisconnected: false,
        },
        [ready.customAudienceId]: {
          lower: 50,
          upper: 60,
          deliveryStatus: null,
          ready: true,
          missingOnMeta: false,
          pageDisconnected: false,
        },
        [disconnected.customAudienceId]: {
          lower: null,
          upper: null,
          deliveryStatus: { code: 300, description: "Too small" },
          ready: false,
          missingOnMeta: false,
          pageDisconnected: true,
        },
        [missing.customAudienceId]: {
          lower: null,
          upper: null,
          deliveryStatus: null,
          ready: false,
          missingOnMeta: true,
          pageDisconnected: false,
        },
      },
    })

    const [list] = await graphCalls(since, `${actA}/customaudiences`, "GET")
    expect(list?.query).toMatchObject({
      fields:
        "id,name,subtype,description,delivery_status,operation_status,approximate_count_lower_bound,approximate_count_upper_bound,messenger_marketing_messages_page",
      limit: "499",
      access_token: mm.token,
    })
    expect(JSON.parse(String(list?.query.filtering))).toEqual([
      { field: "data_source.subtype", operator: "IN", value: [1010] },
    ])
    // Only the one pending audience still on Meta is token-checked, as the Page.
    const tokens = await graphCalls(since, `${page.pageId}/notification_message_tokens`, "GET")
    expect(tokens).toHaveLength(1)
    expect(tokens[0]?.query).toEqual({
      custom_audience_ids: pending.customAudienceId,
      limit: "1",
      access_token: page.accessToken,
    })

    // The ready state is persisted.
    const stored = (await listAudiences(mm.api)).find((row) => row.id === pending.id)
    expect(Math.abs(msFromNow(stored?.readyAt ?? null))).toBeLessThan(60_000)
  })

  it("isolates a failed ad account read (not reported missing); empty without audiences", async () => {
    const mm = await mmTenant("p108-mm-aud-live-fail")
    const audience = await insertAudience(mm.tenant.workspaceId, { readyAt: new Date() })
    await script({
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${audience.adAccountId}/customaudiences`) },
      respond: { status: 400, json: graphError(100, "Unsupported get request.") },
    })
    const response = await mm.api.get(`${AUDIENCES}/live`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      statuses: {
        [audience.customAudienceId]: {
          lower: null,
          upper: null,
          deliveryStatus: null,
          ready: true,
          missingOnMeta: false,
          pageDisconnected: false,
        },
      },
    })

    const empty = await mmTenant("p108-mm-aud-live-empty")
    const none = await empty.api.get(`${AUDIENCES}/live`)
    expect(none.status).toBe(200)
    expect(none.json).toEqual({ statuses: {} })
  })

  it("is 400 mmGrantInvalid without a grant", async () => {
    const mm = await mmTenant("p108-mm-aud-live-nogrant", { grant: "none", scriptDebugToken: false })
    expectError(await mm.api.get(`${AUDIENCES}/live`), 400, "mmGrantInvalid", { message: CONNECT_FIRST })
  })
})

/** Grant + one ToS-accepted account + a connected Page, and the two Graph writes of a create. */
const createSetup = async (label: string, options: { createStatus?: number; uploadStatus?: number } = {}) => {
  const mm = await mmTenant(label)
  const act = uniqueAct()
  const page = await messengerPage(mm.tenant.workspaceId)
  const customAudienceId = uniqueDigits()
  await scriptAdAccounts([graphAdAccount(act)])
  await script(
    {
      match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${act}/customaudiences`) },
      respond:
        options.createStatus && options.createStatus >= 400
          ? { status: options.createStatus, json: graphError(2654, "Terms of service not accepted") }
          : { status: 200, json: { id: customAudienceId } },
    },
    {
      match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${customAudienceId}/users`) },
      respond:
        options.uploadStatus && options.uploadStatus >= 400
          ? { status: options.uploadStatus, json: graphError(100, "Invalid schema") }
          : { status: 200, json: { audience_id: customAudienceId, num_received: 2, num_invalid_entries: 1 } },
    },
  )
  return { ...mm, act, page, customAudienceId }
}

const TABLE = {
  headers: ["Email", "First Name", "Ignored column"],
  rows: [
    ["  Alice@Example.COM ", "Alice", "x"],
    ["not-an-email", "Bob", "y"],
    ["carol@example.com", "Carol-Ann 2", "z"],
  ],
}

describe("facebookMarketingMessages.audiences.create", () => {
  it("creates the Meta audience, stores it and uploads the hashed rows", async () => {
    const setup = await createSetup("p108-mm-aud-create")
    const since = await fake.cursor()
    const response = await setup.api.post(AUDIENCES, {
      adAccountId: setup.act,
      pageId: setup.page.pageId,
      name: "Parity buyers",
      description: "From the API",
      ...TABLE,
    })
    expect(response.status, response.text).toBe(201)
    const outcome = response.json as Record<string, unknown>
    expect(outcome).toEqual({
      status: "ok",
      id: outcome.id,
      customAudienceId: setup.customAudienceId,
      received: 2,
      skipped: 1,
      invalid: 1,
    })
    expect(String(outcome.id)).toMatch(/^\d{15,20}$/)

    const [create] = await graphCalls(since, `${setup.act}/customaudiences`, "POST")
    expect(create?.query).toEqual({ access_token: setup.token })
    expect(jsonBody(create)).toEqual({
      subtype: "CUSTOM",
      customer_file_source: "USER_PROVIDED_ONLY",
      name: "Parity buyers",
      description: "From the API",
      use_for_products: ["MARKETING_MESSAGES"],
      subscription_info: ["MESSENGER"],
      marketing_message_channels: { MESSENGER: setup.page.pageId },
    })
    const uploads = await graphCalls(since, `${setup.customAudienceId}/users`, "POST")
    expect(uploads).toHaveLength(1)
    expect(uploads[0]?.query).toEqual({ access_token: setup.token })
    const upload = jsonBody(uploads[0]) as {
      payload: unknown
      session: { session_id: unknown; batch_seq: number; last_batch_flag: boolean; estimated_num_total: number }
    }
    expect(upload.payload).toEqual({
      schema: ["EMAIL", "FN"],
      data: [
        [sha("alice@example.com"), sha("alice")],
        [sha("carol@example.com"), sha("carolann")],
      ],
    })
    expect(upload.session).toMatchObject({ batch_seq: 1, last_batch_flag: true, estimated_num_total: 2 })
    expect(Number.isSafeInteger(upload.session.session_id)).toBe(true)

    const [stored] = await listAudiences(setup.api)
    expect(stored).toMatchObject({
      id: outcome.id,
      customAudienceId: setup.customAudienceId,
      adAccountId: setup.act,
      pageId: setup.page.pageId,
      name: "Parity buyers",
      description: "From the API",
      rowsUploaded: 2,
      readyAt: null,
    })
  })

  it("reports `invalid` (201) for anything refused before the Meta create", async () => {
    const setup = await createSetup("p108-mm-aud-create-invalid")
    const body = { adAccountId: setup.act, pageId: setup.page.pageId, name: "Refused", ...TABLE }
    const since = await fake.cursor()
    const cases: Array<[Record<string, unknown>, string]> = [
      [{ headers: ["Name", "City"], rows: [["A", "B"]] }, "The file needs an email or phone column."],
      [{ headers: ["Email"], rows: [["nope"], [""]] }, "No row has a valid email or phone number."],
      [{ pageId: uniqueDigits() }, "The selected Page is not connected to this workspace."],
      [{ adAccountId: uniqueAct() }, "Please grant access to use the Marketing Messages feature."],
    ]
    for (const [override, message] of cases) {
      const response = await setup.api.post(AUDIENCES, { ...body, ...override })
      expect(response.status, response.text).toBe(201)
      expect(response.json).toEqual({ status: "invalid", message })
    }
    expect(await graphCalls(since, `${setup.act}/customaudiences`, "POST")).toHaveLength(0)
    expect(await listAudiences(setup.api)).toEqual([])
  })

  it("reports a missing grant as an `invalid` outcome too (201), with no Graph call", async () => {
    const mm = await mmTenant("p108-mm-aud-create-nogrant", { grant: "none", scriptDebugToken: false })
    const since = await fake.cursor()
    const response = await mm.api.post(AUDIENCES, { adAccountId: uniqueAct(), pageId: uniqueDigits(), name: "X", ...TABLE })
    expect(response.status, response.text).toBe(201)
    expect(response.json).toEqual({ status: "invalid", message: CONNECT_FIRST })
    expect(await fake.requests({ kind: "http", host: GRAPH, since })).toHaveLength(0)
  })

  it("reports a Meta create rejection as `invalid` and an upload failure as `partial`", async () => {
    const rejected = await createSetup("p108-mm-aud-create-rejected", { createStatus: 400 })
    const refused = await rejected.api.post(AUDIENCES, {
      adAccountId: rejected.act,
      pageId: rejected.page.pageId,
      name: "Rejected",
      ...TABLE,
    })
    expect(refused.status, refused.text).toBe(201)
    expect(refused.json).toMatchObject({ status: "invalid" })
    expect((refused.json as { message: string }).message).toMatch(/^Meta rejected the audience: /)
    expect(await listAudiences(rejected.api)).toEqual([])

    const partial = await createSetup("p108-mm-aud-create-partial", { uploadStatus: 400 })
    const response = await partial.api.post(AUDIENCES, {
      adAccountId: partial.act,
      pageId: partial.page.pageId,
      name: "Partial",
      ...TABLE,
    })
    expect(response.status, response.text).toBe(201)
    expect(response.json).toEqual({
      status: "partial",
      received: 0,
      message:
        'The audience was created, but the upload stopped after 0 rows. Use "Upload more rows" to retry.',
    })
    // The audience exists on Meta, so the row is kept for "upload more".
    const [stored] = await listAudiences(partial.api)
    expect(stored).toMatchObject({ customAudienceId: partial.customAudienceId, rowsUploaded: 0, name: "Partial" })
  })

  it("validates the body (422)", async () => {
    const mm = await mmTenant("p108-mm-aud-create-422", { grant: "none", scriptDebugToken: false })
    const body = { adAccountId: uniqueAct(), pageId: uniqueDigits(), name: "X", ...TABLE }
    expectInvalid(await mm.api.post(AUDIENCES, { ...body, headers: [] }), "headers")
    expectInvalid(await mm.api.post(AUDIENCES, { ...body, name: "" }), "name")
    expectInvalid(await mm.api.post(AUDIENCES, { ...body, name: "x".repeat(101) }), "name")
    expectInvalid(await mm.api.post(AUDIENCES, { ...body, description: "x".repeat(501) }), "description")
    expectInvalid(await mm.api.post(AUDIENCES, { ...body, pageId: " " }), "pageId")
    expectInvalid(
      await mm.api.post(AUDIENCES, { ...body, rows: Array.from({ length: 100_001 }, () => ["a@b.co"]) }),
      "rows",
    )
  })
})

describe("facebookMarketingMessages.audiences.uploadMore", () => {
  it("uploads more rows to an existing audience and adds them to rowsUploaded", async () => {
    const mm = await mmTenant("p108-mm-aud-more")
    const audience = await insertAudience(mm.tenant.workspaceId, { rowsUploaded: 10 })
    await script({
      match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${audience.customAudienceId}/users`) },
      respond: { status: 200, json: { num_received: 1, num_invalid_entries: 0 } },
    })
    const since = await fake.cursor()
    const response = await mm.api.post(`${AUDIENCES}/${audience.id}/rows`, {
      headers: ["email"],
      rows: [["dave@example.com"], ["bad"]],
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      status: "ok",
      id: audience.id,
      customAudienceId: audience.customAudienceId,
      received: 1,
      skipped: 1,
      invalid: 0,
    })
    const [upload] = await graphCalls(since, `${audience.customAudienceId}/users`, "POST")
    expect((jsonBody(upload) as { payload: unknown }).payload).toEqual({
      schema: ["EMAIL"],
      data: [[sha("dave@example.com")]],
    })
    const [stored] = await listAudiences(mm.api)
    expect(stored?.rowsUploaded).toBe(11)
  })

  it("reports an unknown audience as `invalid`; 400 mmGrantInvalid without a grant", async () => {
    const mm = await mmTenant("p108-mm-aud-more-unknown")
    const unknown = await mm.api.post(`${AUDIENCES}/123456789012345678/rows`, { headers: ["email"], rows: [["a@b.co"]] })
    expect(unknown.status, unknown.text).toBe(200)
    expect(unknown.json).toEqual({ status: "invalid", message: "Custom audience not found." })

    const noGrant = await mmTenant("p108-mm-aud-more-nogrant", { grant: "none", scriptDebugToken: false })
    const audience = await insertAudience(noGrant.tenant.workspaceId)
    expectError(
      await noGrant.api.post(`${AUDIENCES}/${audience.id}/rows`, { headers: ["email"], rows: [["a@b.co"]] }),
      400,
      "mmGrantInvalid",
      { message: CONNECT_FIRST },
    )
  })

  it("validates the body (422)", async () => {
    const mm = await mmTenant("p108-mm-aud-more-422", { grant: "none", scriptDebugToken: false })
    expectInvalid(await mm.api.post(`${AUDIENCES}/1/rows`, { headers: [], rows: [] }), "headers")
    expectInvalid(await mm.api.post(`${AUDIENCES}/1/rows`, { headers: ["email"] }), "rows")
  })
})

describe("facebookMarketingMessages.audiences.delete", () => {
  it("deletes the audience on Meta, then the row", async () => {
    const mm = await mmTenant("p108-mm-aud-delete")
    const audience = await insertAudience(mm.tenant.workspaceId)
    await script({
      match: { host: GRAPH, method: "DELETE", pathRegex: graphPath(audience.customAudienceId) },
      respond: { status: 200, json: { success: true } },
    })
    const since = await fake.cursor()
    const response = await mm.api.delete(`${AUDIENCES}/${audience.id}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ id: audience.id })
    const calls = await graphCalls(since, audience.customAudienceId, "DELETE")
    expect(calls).toHaveLength(1)
    expect(calls[0]?.query).toEqual({ access_token: mm.token })
    expect(await listAudiences(mm.api)).toEqual([])
  })

  it("removes the row when Meta no longer has the audience (100/33); keeps it on another Meta error", async () => {
    const mm = await mmTenant("p108-mm-aud-delete-meta")
    const gone = await insertAudience(mm.tenant.workspaceId, { name: "Gone" })
    const kept = await insertAudience(mm.tenant.workspaceId, { name: "Kept" })
    await script(
      {
        match: { host: GRAPH, method: "DELETE", pathRegex: graphPath(gone.customAudienceId) },
        respond: { status: 400, json: graphError(100, "Object does not exist", 33) },
      },
      {
        match: { host: GRAPH, method: "DELETE", pathRegex: graphPath(kept.customAudienceId) },
        respond: { status: 400, json: graphError(200, "Permissions error") },
      },
    )
    expect((await mm.api.delete(`${AUDIENCES}/${gone.id}`)).status).toBe(200)
    expectError(await mm.api.delete(`${AUDIENCES}/${kept.id}`), 400, "BAD_REQUEST")
    expect((await listAudiences(mm.api)).map((row) => row.name)).toEqual(["Kept"])
  })

  it("refuses an audience a campaign targets, and an unknown one (400 systemError), before Meta", async () => {
    const mm = await mmTenant("p108-mm-aud-delete-refused")
    const audience = await insertAudience(mm.tenant.workspaceId, { readyAt: new Date() })
    await insertCampaign(mm.tenant.workspaceId, {
      status: "completed",
      targetType: "customAudience",
      audienceId: audience.id,
      adAccountId: audience.adAccountId,
      pageId: audience.pageId,
    })
    const since = await fake.cursor()
    expectError(await mm.api.delete(`${AUDIENCES}/${audience.id}`), 400, "systemError", {
      message: "This custom audience is used by 1 marketing message and cannot be deleted.",
    })
    expectError(await mm.api.delete(`${AUDIENCES}/123456789012345678`), 400, "systemError", {
      message: "Custom audience not found.",
    })
    expect(await graphCalls(since, audience.customAudienceId, "DELETE")).toHaveLength(0)
    expect(await listAudiences(mm.api)).toHaveLength(1)
  })

  it("is 400 mmGrantInvalid without a grant", async () => {
    const mm = await mmTenant("p108-mm-aud-delete-nogrant", { grant: "none", scriptDebugToken: false })
    const audience = await insertAudience(mm.tenant.workspaceId)
    expectError(await mm.api.delete(`${AUDIENCES}/${audience.id}`), 400, "mmGrantInvalid", { message: CONNECT_FIRST })
  })
})
