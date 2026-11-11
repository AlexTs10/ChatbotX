// Meta Conversions API routes of an Instagram channel:
// setCapiDataset (PUT .../capi/dataset), disconnectCapi (DELETE .../capi),
// setCapiTestEventCode (PUT .../capi/test-event-code) and sendCapiTestEvent
// (POST .../capi/test-event). Only Instagram-via-Facebook-Login accounts have
// CAPI; an Instagram-Login account is "not found" for every CAPI route.
// Meta calls go to graph.facebook.com (v24.0): `GET /<datasetId>?fields=id`
// (validate), `POST /<igId>/dataset` (create / reuse), `POST /<datasetId>/events`.
// Dataset and account ids are unique per test, so every script is test-owned.

import { describe, expect, it } from "vitest"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import {
  type Api,
  connectInstagram,
  expectError,
  expectInvalid,
  expectNotFound,
  graphPath,
  type IgAccount,
  newTenant,
  parseBody,
  UNKNOWN_ID,
  uniqueDigits,
  upstreamCalls,
} from "./helpers.ts"

const HOST = "graph.facebook.com"
const NOT_FOUND = "Channel not found"
const META_ERROR = {
  error: {
    message: "(#200) App does not have instagram_manage_events permission on the Instagram Account",
    type: "OAuthException",
    code: 200,
    error_subcode: 2804079,
    fbtrace_id: "parity-trace",
  },
}
const META_MESSAGE = "(#200 - 2804079) App does not have instagram_manage_events permission on the Instagram Account"

type Summary = {
  datasetId: string | null
  capiConnected: boolean
  capiDisconnected: boolean
  capiTestEventCode: string | null
  hasCapiScope: boolean
}

const summary = async (api: Api, account: IgAccount): Promise<Summary> => {
  const response = await api.get<Summary>(`/v1/instagram-channels/${account.integrationId}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

const capi = (account: IgAccount, rest = "") => `/v1/instagram-channels/${account.integrationId}/capi${rest}`

const answerDatasetRead = (datasetId: string, status = 200) =>
  fake.script({
    match: { host: HOST, method: "GET", pathRegex: graphPath(datasetId) },
    respond: status === 200 ? { status, json: { id: datasetId } } : { status, json: META_ERROR },
  })

const answerDatasetCreate = (account: IgAccount, datasetId: string) =>
  fake.script({
    match: { host: HOST, method: "POST", pathRegex: graphPath(`${account.igId}/dataset`) },
    respond: { status: 200, json: { id: datasetId } },
  })

const answerEvents = (datasetId: string, status = 200) =>
  fake.script({
    match: { host: HOST, method: "POST", pathRegex: graphPath(`${datasetId}/events`) },
    respond: status === 200 ? { status, json: { events_received: 1, fbtrace_id: "x" } } : { status, json: META_ERROR },
  })

const accountCalls = async (account: IgAccount, since: number, datasetIds: string[]): Promise<UpstreamRecord[]> => {
  const records = await upstreamCalls({ host: HOST, since })
  return records.filter((record) =>
    [account.igId, ...datasetIds].some((id) => record.path.includes(`/${id}`)),
  )
}

const facebookAccount = (tenant: Tenant, extra: Parameters<typeof connectInstagram>[2] = {}) =>
  connectInstagram(tenant, "facebook", extra)

describe("instagramChannels.setCapiDataset", () => {
  it("validates the dataset with the channel token, stores it and clears a disconnect", async () => {
    const { tenant, api } = await newTenant("p110-capi-ds")
    const account = await facebookAccount(tenant, { hasCapiScope: true, capiDisconnected: true })
    const datasetId = uniqueDigits()
    await answerDatasetRead(datasetId)
    const since = await fake.cursor()
    const response = await api.put(capi(account, "/dataset"), { datasetId: `  ${datasetId}  ` })
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")

    const calls = await accountCalls(account, since, [datasetId])
    expect(calls).toHaveLength(1)
    expect(calls[0]).toMatchObject({ method: "GET", query: { fields: "id" } })
    expect(calls[0]?.path).toMatch(new RegExp(graphPath(datasetId)))
    expect(calls[0]?.path.startsWith("/v24.0/")).toBe(true)
    expect(calls[0]?.headers.authorization).toBe(`Bearer ig-token-${account.igId}`)
    expect(await summary(api, account)).toMatchObject({
      datasetId,
      capiConnected: true,
      capiDisconnected: false,
      hasCapiScope: true,
    })
  })

  it("without the CAPI scope the dataset is stored but capiConnected stays false", async () => {
    const { tenant, api } = await newTenant("p110-capi-ds-noscope")
    const account = await facebookAccount(tenant)
    const datasetId = uniqueDigits()
    await answerDatasetRead(datasetId)
    expect((await api.put(capi(account, "/dataset"), { datasetId })).status).toBe(204)
    expect(await summary(api, account)).toMatchObject({ datasetId, capiConnected: false, capiDisconnected: false })
  })

  it("a Meta rejection is 400 capiRequestFailed with Meta's message; nothing changes", async () => {
    const { tenant, api } = await newTenant("p110-capi-ds-reject")
    const account = await facebookAccount(tenant, { datasetId: "kept-dataset", hasCapiScope: true, capiDisconnected: true })
    const datasetId = uniqueDigits()
    await answerDatasetRead(datasetId, 400)
    expectError(await api.put(capi(account, "/dataset"), { datasetId }), 400, "capiRequestFailed", {
      defined: true,
      message: META_MESSAGE,
    })
    expect(await summary(api, account)).toMatchObject({ datasetId: "kept-dataset", capiDisconnected: true })
  })

  it("without datasetId creates (or reuses) the account's dataset at Meta and stores it", async () => {
    const { tenant, api } = await newTenant("p110-capi-ds-create")
    const account = await facebookAccount(tenant, { datasetId: "old-dataset", hasCapiScope: true })
    const created = uniqueDigits()
    await answerDatasetCreate(account, created)
    const since = await fake.cursor()
    const response = await api.put(capi(account, "/dataset"), {})
    expect(response.status, response.text).toBe(204)
    const calls = await accountCalls(account, since, [created])
    expect(calls.map((call) => [call.method, call.path.replace(/^\/v[0-9.]+/, "")])).toEqual([
      ["POST", `/${account.igId}/dataset`],
    ])
    expect(parseBody(calls[0] as UpstreamRecord)).toEqual({ dataset_name: `${account.name} Event Data` })
    expect(calls[0]?.headers.authorization).toBe(`Bearer ig-token-${account.igId}`)
    expect(await summary(api, account)).toMatchObject({ datasetId: created, capiConnected: true })
  })

  it("is 404 for an Instagram-Login account and for an unknown or foreign id; 422 for an empty datasetId", async () => {
    const { tenant, api } = await newTenant("p110-capi-ds-404")
    const native = await connectInstagram(tenant, "instagram")
    const since = await fake.cursor()
    expectNotFound(await api.put(capi(native, "/dataset"), { datasetId: "123" }), NOT_FOUND)
    expectNotFound(await api.put(`/v1/instagram-channels/${UNKNOWN_ID}/capi/dataset`, { datasetId: "123" }), NOT_FOUND)
    const { api: otherApi } = await newTenant("p110-capi-ds-404-o")
    const account = await facebookAccount(tenant)
    expectNotFound(await otherApi.put(capi(account, "/dataset"), {}), NOT_FOUND)
    expect(await accountCalls(native, since, [])).toEqual([])
    expectInvalid(await api.put(capi(account, "/dataset"), { datasetId: "   " }), "datasetId")
    expectInvalid(await api.put("/v1/instagram-channels/abc/capi/dataset", {}), "id")
  })
})

describe("instagramChannels.disconnectCapi", () => {
  it("marks CAPI disconnected and keeps dataset and test code; setCapiDataset reconnects", async () => {
    const { tenant, api } = await newTenant("p110-capi-off")
    const datasetId = uniqueDigits()
    const account = await facebookAccount(tenant, { datasetId, hasCapiScope: true, capiTestEventCode: "KEEP1" })
    expect((await summary(api, account)).capiConnected).toBe(true)
    const since = await fake.cursor()
    const response = await api.delete(capi(account))
    expect(response.status, response.text).toBe(204)
    expect(await summary(api, account)).toMatchObject({
      datasetId,
      capiTestEventCode: "KEEP1",
      capiConnected: false,
      capiDisconnected: true,
    })
    expect(await accountCalls(account, since, [datasetId])).toEqual([])
    // A second disconnect is fine.
    expect((await api.delete(capi(account))).status).toBe(204)

    await answerDatasetRead(datasetId)
    expect((await api.put(capi(account, "/dataset"), { datasetId })).status).toBe(204)
    expect(await summary(api, account)).toMatchObject({ capiConnected: true, capiDisconnected: false })
  })

  it("is 404 for an Instagram-Login account or an unknown id", async () => {
    const { tenant, api } = await newTenant("p110-capi-off-404")
    const native = await connectInstagram(tenant, "instagram")
    expectNotFound(await api.delete(capi(native)), NOT_FOUND)
    expectNotFound(await api.delete(`/v1/instagram-channels/${UNKNOWN_ID}/capi`), NOT_FOUND)
  })
})

describe("instagramChannels.setCapiTestEventCode", () => {
  it("sets (trimmed), replaces and clears the code with null or an empty string", async () => {
    const { tenant, api } = await newTenant("p110-capi-code")
    const account = await facebookAccount(tenant)
    const path = capi(account, "/test-event-code")
    const set = await api.put(path, { testEventCode: "  TEST123  " })
    expect(set.status, set.text).toBe(204)
    expect((await summary(api, account)).capiTestEventCode).toBe("TEST123")
    expect((await api.put(path, { testEventCode: "TEST_9-x" })).status).toBe(204)
    expect((await summary(api, account)).capiTestEventCode).toBe("TEST_9-x")
    expect((await api.put(path, { testEventCode: null })).status).toBe(204)
    expect((await summary(api, account)).capiTestEventCode).toBeNull()
    expect((await api.put(path, { testEventCode: "AGAIN" })).status).toBe(204)
    expect((await api.put(path, { testEventCode: "" })).status).toBe(204)
    expect((await summary(api, account)).capiTestEventCode).toBeNull()
  })

  it("validates the code (422) and is 404 for an Instagram-Login account", async () => {
    const { tenant, api } = await newTenant("p110-capi-code-bad")
    const account = await facebookAccount(tenant)
    const path = capi(account, "/test-event-code")
    expectInvalid(await api.put(path, { testEventCode: "has space" }), "testEventCode")
    expectInvalid(await api.put(path, { testEventCode: "x".repeat(65) }), "testEventCode")
    expectInvalid(await api.put(path, {}), "testEventCode")
    const native = await connectInstagram(tenant, "instagram")
    expectNotFound(await api.put(capi(native, "/test-event-code"), { testEventCode: "T1" }), NOT_FOUND)
  })
})

describe("instagramChannels.sendCapiTestEvent", () => {
  it("posts one sample Purchase for the messaging id to the dataset's events, with the test code", async () => {
    const { tenant, api } = await newTenant("p110-capi-event")
    const datasetId = uniqueDigits()
    const account = await facebookAccount(tenant, { datasetId, hasCapiScope: true, capiTestEventCode: "TEST777" })
    await answerEvents(datasetId)
    const since = await fake.cursor()
    const before = Math.floor(Date.now() / 1000)
    const response = await api.post(capi(account, "/test-event"), { messagingId: "  IGSID_123-abc  " })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ success: true })

    const calls = await accountCalls(account, since, [datasetId])
    expect(calls).toHaveLength(1)
    const [call] = calls as [UpstreamRecord]
    expect(call.method).toBe("POST")
    expect(call.path).toBe(`/v24.0/${datasetId}/events`)
    expect(call.headers.authorization).toBe(`Bearer ig-token-${account.igId}`)
    const body = parseBody<{ data: Array<Record<string, unknown>>; partner_agent: string; test_event_code: string }>(call)
    expect(Object.keys(body).sort()).toEqual(["data", "partner_agent", "test_event_code"])
    expect(body.partner_agent).toBe("ChatConnectX")
    expect(body.test_event_code).toBe("TEST777")
    expect(body.data).toHaveLength(1)
    const [event] = body.data as [Record<string, unknown>]
    expect(event).toEqual({
      event_name: "Purchase",
      event_time: expect.any(Number),
      event_id: expect.stringMatching(/^test:\d{15,20}$/),
      action_source: "business_messaging",
      messaging_channel: "instagram",
      user_data: {
        ig_account_id: account.igId,
        instagram_business_account_id: account.igId,
        ig_sid: "IGSID_123-abc",
      },
      custom_data: { currency: "USD", value: 100 },
    })
    expect(event.event_time as number).toBeGreaterThanOrEqual(before - 5)
    expect(event.event_time as number).toBeLessThanOrEqual(Math.floor(Date.now() / 1000) + 5)
  })

  it("without a dataset creates one at Meta first, stores it, then sends", async () => {
    const { tenant, api } = await newTenant("p110-capi-event-provision")
    const account = await facebookAccount(tenant, { hasCapiScope: true, capiTestEventCode: "TESTP" })
    const created = uniqueDigits()
    await answerDatasetCreate(account, created)
    await answerEvents(created)
    const since = await fake.cursor()
    const response = await api.post(capi(account, "/test-event"), { messagingId: "IGSID1" })
    expect(response.status, response.text).toBe(200)
    const calls = await accountCalls(account, since, [created])
    expect(calls.map((call) => call.path.replace(/^\/v[0-9.]+/, ""))).toEqual([`/${account.igId}/dataset`, `/${created}/events`])
    expect((await summary(api, account)).datasetId).toBe(created)
  })

  it("refuses without a test code, when disconnected, or for a bad messaging id (422 capiTestEventRefused)", async () => {
    const { tenant, api } = await newTenant("p110-capi-event-refused")
    const datasetId = uniqueDigits()
    const noCode = await facebookAccount(tenant, { datasetId, hasCapiScope: true })
    const off = await facebookAccount(tenant, { datasetId, hasCapiScope: true, capiTestEventCode: "T1", capiDisconnected: true })
    const ok = await facebookAccount(tenant, { datasetId, hasCapiScope: true, capiTestEventCode: "T1" })
    const since = await fake.cursor()
    expectError(await api.post(capi(noCode, "/test-event"), { messagingId: "IGSID1" }), 422, "capiTestEventRefused", {
      defined: true,
      message:
        "Save a test event code first (`setCapiTestEventCode` of the same channel group), then send a test event.",
    })
    expectError(await api.post(capi(off, "/test-event"), { messagingId: "IGSID1" }), 422, "capiTestEventRefused", {
      defined: true,
      message: "Conversions API is disconnected for this channel. Save a dataset to reconnect it.",
    })
    expectError(await api.post(capi(ok, "/test-event"), { messagingId: "not valid!" }), 422, "capiTestEventRefused", {
      defined: true,
      message: "`messagingId` is not a valid messaging id.",
    })
    expect(await accountCalls(ok, since, [datasetId])).toEqual([])
    expectInvalid(await api.post(capi(ok, "/test-event"), { messagingId: "   " }), "messagingId")
    expectInvalid(await api.post(capi(ok, "/test-event"), { messagingId: "x".repeat(513) }), "messagingId")
    expectInvalid(await api.post(capi(ok, "/test-event"), {}), "messagingId")
  })

  it("a Meta rejection of the event is 400 capiRequestFailed", async () => {
    const { tenant, api } = await newTenant("p110-capi-event-reject")
    const datasetId = uniqueDigits()
    const account = await facebookAccount(tenant, { datasetId, hasCapiScope: true, capiTestEventCode: "T2" })
    await answerEvents(datasetId, 400)
    expectError(await api.post(capi(account, "/test-event"), { messagingId: "IGSID2" }), 400, "capiRequestFailed", {
      defined: true,
      message: META_MESSAGE,
    })
  })

  it("is 404 for an Instagram-Login account or an unknown id", async () => {
    const { tenant, api } = await newTenant("p110-capi-event-404")
    const native = await connectInstagram(tenant, "instagram")
    expectNotFound(await api.post(capi(native, "/test-event"), { messagingId: "IGSID1" }), NOT_FOUND)
    expectNotFound(await api.post(`/v1/instagram-channels/${UNKNOWN_ID}/capi/test-event`, { messagingId: "IGSID1" }), NOT_FOUND)
  })
})
