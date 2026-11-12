// messengerChannels: Meta Conversions API (setCapiDataset, disconnectCapi,
// setCapiTestEventCode, sendCapiTestEvent), coexist history sync
// (getCoexistStatus, setCoexist), updateTagSync, updateHandoverResumeFlow
// and disconnect (scope `channels`).
// Meta calls (graph.facebook.com, page token unless noted):
//   CAPI     `GET /v24.0/<datasetId>?fields=id` (validate), `POST /<pageId>/dataset`
//            (create / reuse), `POST /v24.0/<datasetId>/events`;
//   coexist  the `scanCoexistRuns` schedule (every minute) pulls
//            `GET /v23.0/<pageId>/conversations` (contacts phase, then messages phase);
//   disconnect `DELETE /v23.0/me/messenger_profile` {fields:[persistent_menu]},
//            then `DELETE /v23.0/<pageId>/subscribed_apps` with the app token
//            `<clientId>|<clientSecret>`; when an Instagram account of the same
//            page and app still exists, instead `POST /v23.0/me/subscribed_apps`
//            {subscribed_fields: general_info}.
// Dataset, page and comment ids are unique per test; the shared `/me/...`
// answers carry no page data (unlimited scripts set once in this file).

import { beforeAll, describe, expect, it } from "vitest"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import {
  type Api,
  connectMessenger,
  createPublishedFlow,
  expectError,
  expectInvalid,
  expectNotFound,
  ext,
  GRAPH,
  graphError,
  graphPath,
  ISO,
  type MessengerPage,
  newTenant,
  parseBody,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueDigits,
  uniqueName,
  upstreamCalls,
  wait,
} from "./helpers.ts"

beforeAll(async () => {
  await fake.script(
    {
      match: { host: GRAPH, method: "DELETE", pathRegex: graphPath("me/messenger_profile") },
      respond: { status: 200, json: { result: "success" } },
    },
    {
      match: { host: GRAPH, method: "POST", pathRegex: graphPath("me/subscribed_apps"), bodyIncludes: "general_info" },
      respond: { status: 200, json: { success: true } },
    },
  )
})

type Summary = {
  datasetId: string | null
  capiConnected: boolean
  capiDisconnected: boolean
  capiTestEventCode: string | null
  hasCapiScope: boolean
  coexistEnabled: boolean
  syncTagEnabledAt: string | null
  handoverResumeFlowId: string | null
}

const summary = async (api: Api, page: MessengerPage): Promise<Summary> => {
  const response = await api.get<Summary>(`/v1/messenger-channels/${page.integrationId}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

const capi = (page: MessengerPage, rest = "") => `/v1/messenger-channels/${page.integrationId}/capi${rest}`

const META_ERROR = {
  error: {
    message: "(#200) App does not have page_events permission",
    type: "OAuthException",
    code: 200,
    error_subcode: 2804079,
    fbtrace_id: "parity-trace",
  },
}
const META_MESSAGE = "(#200 - 2804079) App does not have page_events permission"

const answerDatasetRead = (datasetId: string, status = 200) =>
  fake.script({
    match: { host: GRAPH, method: "GET", pathRegex: graphPath(datasetId) },
    respond: status === 200 ? { status, json: { id: datasetId } } : { status, json: META_ERROR },
  })

const answerDatasetCreate = (page: MessengerPage, datasetId: string) =>
  fake.script({
    match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${page.pageId}/dataset`) },
    respond: { status: 200, json: { id: datasetId } },
  })

const answerEvents = (datasetId: string, status = 200) =>
  fake.script({
    match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${datasetId}/events`) },
    respond: status === 200 ? { status, json: { events_received: 1, fbtrace_id: "x" } } : { status, json: META_ERROR },
  })

const pageCalls = async (page: MessengerPage, since: number, ids: string[] = []): Promise<UpstreamRecord[]> =>
  (await upstreamCalls({ host: GRAPH, since })).filter((record) =>
    [page.pageId, ...ids].some((id) => record.path.includes(`/${id}`)) ||
    record.headers.authorization === `Bearer ${page.accessToken}`,
  )

describe("messengerChannels CAPI", () => {
  it("setCapiDataset validates the dataset with the page token, stores it and clears a disconnect", async () => {
    const { tenant, api } = await newTenant("p111-capi-ds")
    const page = await connectMessenger(tenant, { hasCapiScope: true, capiDisconnected: true })
    const datasetId = uniqueDigits()
    await answerDatasetRead(datasetId)
    const since = await fake.cursor()
    const response = await api.put(capi(page, "/dataset"), { datasetId: `  ${datasetId}  ` })
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    const calls = await pageCalls(page, since, [datasetId])
    expect(calls).toHaveLength(1)
    expect(calls[0]).toMatchObject({ method: "GET", path: `/v24.0/${datasetId}`, query: { fields: "id" } })
    expect(calls[0]?.headers.authorization).toBe(`Bearer ${page.accessToken}`)
    expect(await summary(api, page)).toMatchObject({ datasetId, capiConnected: true, capiDisconnected: false })
  })

  it("setCapiDataset: a Meta rejection is 400 capiRequestFailed; without datasetId the page dataset is created", async () => {
    const { tenant, api } = await newTenant("p111-capi-ds-create")
    const page = await connectMessenger(tenant, { datasetId: "kept", hasCapiScope: true, capiDisconnected: true })
    const rejected = uniqueDigits()
    await answerDatasetRead(rejected, 400)
    expectError(await api.put(capi(page, "/dataset"), { datasetId: rejected }), 400, "capiRequestFailed", {
      defined: true,
      message: META_MESSAGE,
    })
    expect(await summary(api, page)).toMatchObject({ datasetId: "kept", capiDisconnected: true })

    const created = uniqueDigits()
    await answerDatasetCreate(page, created)
    const since = await fake.cursor()
    expect((await api.put(capi(page, "/dataset"), {})).status).toBe(204)
    const calls = await pageCalls(page, since, [created])
    expect(calls.map((call) => [call.method, call.path.replace(/^\/v[0-9.]+/, "")])).toEqual([
      ["POST", `/${page.pageId}/dataset`],
    ])
    expect(parseBody(calls[0] as UpstreamRecord)).toEqual({ dataset_name: `${page.name} Event Data` })
    expect(await summary(api, page)).toMatchObject({ datasetId: created, capiConnected: true, capiDisconnected: false })
  })

  it("disconnectCapi keeps dataset and code; setCapiTestEventCode sets, trims and clears", async () => {
    const { tenant, api } = await newTenant("p111-capi-off")
    const datasetId = uniqueDigits()
    const page = await connectMessenger(tenant, { datasetId, hasCapiScope: true, capiTestEventCode: "KEEP1" })
    expect((await summary(api, page)).capiConnected).toBe(true)
    const since = await fake.cursor()
    expect((await api.delete(capi(page))).status).toBe(204)
    expect(await summary(api, page)).toMatchObject({
      datasetId,
      capiTestEventCode: "KEEP1",
      capiConnected: false,
      capiDisconnected: true,
    })
    expect(await pageCalls(page, since, [datasetId])).toEqual([])
    expect((await api.delete(capi(page))).status).toBe(204)

    const code = capi(page, "/test-event-code")
    expect((await api.put(code, { testEventCode: "  TEST123  " })).status).toBe(204)
    expect((await summary(api, page)).capiTestEventCode).toBe("TEST123")
    expect((await api.put(code, { testEventCode: "" })).status).toBe(204)
    expect((await summary(api, page)).capiTestEventCode).toBeNull()
    expect((await api.put(code, { testEventCode: "T_9-x" })).status).toBe(204)
    expect((await api.put(code, { testEventCode: null })).status).toBe(204)
    expect((await summary(api, page)).capiTestEventCode).toBeNull()
    expectInvalid(await api.put(code, { testEventCode: "has space" }), "testEventCode")
    expectInvalid(await api.put(code, { testEventCode: "x".repeat(65) }), "testEventCode")
    expectInvalid(await api.put(code, {}), "testEventCode")
  })

  it("sendCapiTestEvent posts one sample Purchase with page_id + page_scoped_user_id and the test code", async () => {
    const { tenant, api } = await newTenant("p111-capi-event")
    const datasetId = uniqueDigits()
    const page = await connectMessenger(tenant, { datasetId, hasCapiScope: true, capiTestEventCode: "TEST777" })
    await answerEvents(datasetId)
    const since = await fake.cursor()
    const before = Math.floor(Date.now() / 1000)
    const response = await api.post(capi(page, "/test-event"), { messagingId: "  PSID_123  " })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ success: true })
    const calls = await pageCalls(page, since, [datasetId])
    expect(calls).toHaveLength(1)
    const [call] = calls as [UpstreamRecord]
    expect(call).toMatchObject({ method: "POST", path: `/v24.0/${datasetId}/events` })
    expect(call.headers.authorization).toBe(`Bearer ${page.accessToken}`)
    const body = parseBody<{ data: Array<Record<string, unknown>>; partner_agent: string; test_event_code: string }>(call)
    expect(Object.keys(body).sort()).toEqual(["data", "partner_agent", "test_event_code"])
    expect(body).toMatchObject({ partner_agent: "ChatConnectX", test_event_code: "TEST777" })
    expect(body.data).toEqual([
      {
        event_name: "Purchase",
        event_time: expect.any(Number),
        event_id: expect.stringMatching(/^test:\d{15,20}$/),
        action_source: "business_messaging",
        messaging_channel: "messenger",
        user_data: { page_id: page.pageId, page_scoped_user_id: "PSID_123" },
        custom_data: { currency: "USD", value: 100 },
      },
    ])
    expect(body.data[0]?.event_time as number).toBeGreaterThanOrEqual(before - 5)
  })

  it("sendCapiTestEvent refusals (422 capiTestEventRefused), Meta rejection (400) and 404 for unknown pages", async () => {
    const { tenant, api } = await newTenant("p111-capi-event-err")
    const datasetId = uniqueDigits()
    const noCode = await connectMessenger(tenant, { datasetId, hasCapiScope: true })
    const off = await connectMessenger(tenant, { datasetId, hasCapiScope: true, capiTestEventCode: "T1", capiDisconnected: true })
    const ok = await connectMessenger(tenant, { datasetId, hasCapiScope: true, capiTestEventCode: "T1" })
    expectError(await api.post(capi(noCode, "/test-event"), { messagingId: "PSID1" }), 422, "capiTestEventRefused", {
      defined: true,
      message:
        "Save a test event code first (`setCapiTestEventCode` of the same channel group), then send a test event.",
    })
    expectError(await api.post(capi(off, "/test-event"), { messagingId: "PSID1" }), 422, "capiTestEventRefused", {
      defined: true,
      message: "Conversions API is disconnected for this channel. Save a dataset to reconnect it.",
    })
    expectError(await api.post(capi(ok, "/test-event"), { messagingId: "not valid!" }), 422, "capiTestEventRefused", {
      defined: true,
      message: "`messagingId` is not a valid messaging id.",
    })
    expectInvalid(await api.post(capi(ok, "/test-event"), {}), "messagingId")
    await answerEvents(datasetId, 400)
    expectError(await api.post(capi(ok, "/test-event"), { messagingId: "PSID2" }), 400, "capiRequestFailed", {
      defined: true,
      message: META_MESSAGE,
    })
    for (const [method, rest, body] of [
      ["put", "/dataset", { datasetId: "1" }],
      ["delete", "", undefined],
      ["put", "/test-event-code", { testEventCode: "T1" }],
      ["post", "/test-event", { messagingId: "PSID1" }],
    ] as const) {
      const path = `/v1/messenger-channels/${UNKNOWN_ID}/capi${rest}`
      const response =
        method === "delete" ? await api.delete(path) : await api[method](path, body as Record<string, unknown>)
      expectNotFound(response, "Channel not found")
    }
    const { api: otherApi } = await newTenant("p111-capi-event-err-o")
    expectNotFound(await otherApi.put(capi(ok, "/dataset"), {}), "Channel not found")
  })
})

describe("messengerChannels coexist", () => {
  it("run is null before any sync; 404 for an unknown or foreign page", async () => {
    const { tenant, api } = await newTenant("p111-coexist-none")
    const page = await connectMessenger(tenant)
    const status = await api.get(`/v1/messenger-channels/${page.integrationId}/coexist`)
    expect(status.status, status.text).toBe(200)
    expect(status.json).toEqual({ run: null })
    expectNotFound(await api.get(`/v1/messenger-channels/${UNKNOWN_ID}/coexist`), "Channel not found")
    const { api: otherApi } = await newTenant("p111-coexist-none-o")
    expectNotFound(await otherApi.get(`/v1/messenger-channels/${page.integrationId}/coexist`), "Channel not found")
    expectInvalid(await api.get("/v1/messenger-channels/abc/coexist"), "id")
  })

  it(
    "enable returns the run id (reused while live); the scheduler runs both phases to succeeded",
    async () => {
      const { tenant, api } = await newTenant("p111-coexist-on")
      const page = await connectMessenger(tenant)
      await fake.script({
        match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${page.pageId}/conversations`) },
        respond: { status: 200, json: { data: [] } },
      })
      const path = `/v1/messenger-channels/${page.integrationId}/coexist`
      const since = await fake.cursor()
      const enabled = await api.put<{ success: boolean; runId: string }>(path, { enabled: true })
      expect(enabled.status, enabled.text).toBe(200)
      expect(enabled.json).toEqual({ success: true, runId: expect.stringMatching(SNOWFLAKE) })
      const again = await api.put<{ runId: string }>(path, { enabled: true, aiReadsSyncedHistory: true })
      expect(again.json).toEqual({ success: true, runId: enabled.json.runId })
      expect((await summary(api, page)).coexistEnabled).toBe(true)

      type Run = { id: string; status: string; finishedAt: string | null; currentStep: string | null }
      const done = await wait(async () => {
        const response = await api.get<{ run: Run }>(path)
        expect(response.json.run?.status).toBe("succeeded")
        return response.json.run
      }, 110_000)
      expect(done).toMatchObject({
        id: enabled.json.runId,
        importedContactCount: 0,
        importedMessageCount: 0,
        skippedCount: 0,
        failedCount: 0,
        currentStep: "done",
      })
      expect(done.finishedAt).toMatch(ISO)
      const pulls = await upstreamCalls({
        host: GRAPH,
        since,
        path: new RegExp(graphPath(`${page.pageId}/conversations`)),
      })
      expect(pulls.length).toBeGreaterThanOrEqual(2)
      expect(pulls[0]?.headers.authorization).toBe(`Bearer ${page.accessToken}`)
      expect(pulls[0]?.query).toMatchObject({ fields: "id,participants,updated_time", platform: "MESSENGER", limit: "499" })
    },
    150_000,
  )

  it("disable turns coexist off (no run id); unknown pages 404; the body is validated", async () => {
    const { tenant, api } = await newTenant("p111-coexist-off")
    const page = await connectMessenger(tenant)
    const path = `/v1/messenger-channels/${page.integrationId}/coexist`
    const response = await api.put(path, { enabled: false })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ success: true })
    expect((await summary(api, page)).coexistEnabled).toBe(false)
    expectNotFound(await api.put(`/v1/messenger-channels/${UNKNOWN_ID}/coexist`, { enabled: true }), "Channel not found")
    expectNotFound(await api.put(`/v1/messenger-channels/${UNKNOWN_ID}/coexist`, { enabled: false }), "Channel not found")
    expectInvalid(await api.put(path, {}), "enabled")
  })
})

describe("messengerChannels.updateTagSync / updateHandoverResumeFlow", () => {
  it("tag sync on returns and stores the time; off clears it; unknown page 404", async () => {
    const { tenant, api } = await newTenant("p111-tagsync")
    const page = await connectMessenger(tenant)
    const path = `/v1/messenger-channels/${page.integrationId}/tag-sync`
    const before = Date.now()
    const on = await api.patch<{ syncTagEnabledAt: string }>(path, { enabled: true })
    expect(on.status, on.text).toBe(200)
    expect(on.json.syncTagEnabledAt).toMatch(ISO)
    expect(Date.parse(on.json.syncTagEnabledAt)).toBeGreaterThanOrEqual(before - 5_000)
    expect((await summary(api, page)).syncTagEnabledAt).toBe(on.json.syncTagEnabledAt)
    const off = await api.patch(path, { enabled: false })
    expect(off.json).toEqual({ syncTagEnabledAt: null })
    expect((await summary(api, page)).syncTagEnabledAt).toBeNull()
    expectNotFound(await api.patch(`/v1/messenger-channels/${UNKNOWN_ID}/tag-sync`, { enabled: true }), "Messenger channel not found")
    expectInvalid(await api.patch(path, {}), "enabled")
  })

  it("handover resume flow: set to an active flow, clear with null; unknown flow or page 404", async () => {
    const { tenant, api } = await newTenant("p111-handover")
    const page = await connectMessenger(tenant)
    const flowId = await createPublishedFlow(api, uniqueName("Resume"), "Welcome back")
    const path = `/v1/messenger-channels/${page.integrationId}/handover-resume-flow`
    const set = await api.patch(path, { handoverResumeFlowId: flowId })
    expect(set.status, set.text).toBe(204)
    expect(set.text).toBe("")
    expect((await summary(api, page)).handoverResumeFlowId).toBe(flowId)
    expect((await api.patch(path, { handoverResumeFlowId: null })).status).toBe(204)
    expect((await summary(api, page)).handoverResumeFlowId).toBeNull()
    expectNotFound(await api.patch(path, { handoverResumeFlowId: UNKNOWN_ID }), "Handover flow not found")
    expectNotFound(
      await api.patch(`/v1/messenger-channels/${UNKNOWN_ID}/handover-resume-flow`, { handoverResumeFlowId: null }),
      "Messenger integration not found",
    )
    const { api: otherApi } = await newTenant("p111-handover-o")
    const foreignFlow = await createPublishedFlow(otherApi, uniqueName("Foreign"), "x")
    expectNotFound(await api.patch(path, { handoverResumeFlowId: foreignFlow }), "Handover flow not found")
    expectInvalid(await api.patch(path, {}), "handoverResumeFlowId")
  })
})

const unsubscribeScript = (page: MessengerPage, respond: { status: number; json: unknown }) =>
  fake.script({ match: { host: GRAPH, method: "DELETE", pathRegex: graphPath(`${page.pageId}/subscribed_apps`) }, respond })

const expectGone = async (api: Api, page: MessengerPage) => {
  expectNotFound(await api.get(`/v1/messenger-channels/${page.integrationId}`), "Channel integration not found")
  expect((await api.get("/v1/messenger-channels")).json).toEqual([])
  const inboxes = await api.get<{ data: Array<{ id: string }> }>("/v1/inboxes")
  expect(inboxes.json.data.map((inbox) => inbox.id)).not.toContain(page.inboxId)
}

describe("messengerChannels.disconnect", () => {
  it("clears the persistent menu, unsubscribes the page with the app token, then removes the channel", async () => {
    const { tenant, api } = await newTenant("p111-mc-disc")
    const page = await connectMessenger(tenant)
    await unsubscribeScript(page, { status: 200, json: { success: true } })
    const since = await fake.cursor()
    const response = await api.delete(`/v1/messenger-channels/${page.integrationId}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    const menu = await upstreamCalls({
      host: GRAPH,
      since,
      path: new RegExp(graphPath("me/messenger_profile")),
      method: "DELETE",
      bearer: page.accessToken,
    })
    expect(menu.map((call) => parseBody(call))).toEqual([{ fields: ["persistent_menu"] }])
    const unsubscribe = await upstreamCalls({
      host: GRAPH,
      since,
      path: new RegExp(graphPath(`${page.pageId}/subscribed_apps`)),
      method: "DELETE",
    })
    expect(unsubscribe).toHaveLength(1)
    expect(unsubscribe[0]?.headers.authorization).toBe("Bearer parity-client|parity-secret")
    await expectGone(api, page)
    expectNotFound(await api.delete(`/v1/messenger-channels/${page.integrationId}`), "Messenger channel not found")
  })

  it("a page still used by an Instagram account keeps a general_info subscription instead", async () => {
    const { tenant, api } = await newTenant("p111-mc-disc-ig")
    const page = await connectMessenger(tenant)
    await (await ext()).createInstagramForPage({
      workspaceId: tenant.workspaceId,
      pageId: page.pageId,
      igId: uniqueDigits(),
      name: uniqueName("IG"),
    })
    const since = await fake.cursor()
    expect((await api.delete(`/v1/messenger-channels/${page.integrationId}`)).status).toBe(204)
    const calls = await upstreamCalls({ host: GRAPH, since, bearer: page.accessToken })
    expect(calls.map((call) => `${call.method} ${call.path.replace(/^\/v[0-9.]+\//, "/v/")}`)).toEqual([
      "POST /v/me/subscribed_apps",
    ])
    expect(parseBody(calls[0] as UpstreamRecord)).toEqual({ subscribed_fields: "general_info" })
    expect(
      await upstreamCalls({ host: GRAPH, since, path: new RegExp(graphPath(`${page.pageId}/subscribed_apps`)) }),
    ).toEqual([])
    expectNotFound(await api.get(`/v1/messenger-channels/${page.integrationId}`), "Channel integration not found")
  })

  it("a Meta error while unsubscribing is only logged: the channel is still removed", async () => {
    const { tenant, api } = await newTenant("p111-mc-disc-error")
    const page = await connectMessenger(tenant)
    await unsubscribeScript(page, { status: 400, json: graphError("(#100) Invalid parameter", 100) })
    const response = await api.delete(`/v1/messenger-channels/${page.integrationId}`)
    expect(response.status, response.text).toBe(204)
    await expectGone(api, page)
  })

  it("404 for an unknown or foreign page; 422 for a non-numeric id", async () => {
    const { tenant, api } = await newTenant("p111-mc-disc-404")
    const page = await connectMessenger(tenant)
    expectNotFound(await api.delete(`/v1/messenger-channels/${UNKNOWN_ID}`), "Messenger channel not found")
    const { api: otherApi } = await newTenant("p111-mc-disc-404-o")
    expectNotFound(await otherApi.delete(`/v1/messenger-channels/${page.integrationId}`), "Messenger channel not found")
    expectInvalid(await api.delete("/v1/messenger-channels/abc"), "id")
    expect((await api.get(`/v1/messenger-channels/${page.integrationId}`)).status).toBe(200)
  })
})
