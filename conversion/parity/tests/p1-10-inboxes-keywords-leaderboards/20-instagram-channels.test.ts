// instagramChannels.list / get (channel summaries, no credentials) and the
// settings routes getSettings / updateSettings (PUT) / patchSettings (PATCH).
// A settings write saves welcome flow, ice breakers and persistent menu and
// pushes them to Instagram: `DELETE /me/messenger_profile` for an empty list,
// `POST /me/messenger_profile` for the others (graph.instagram.com for an
// Instagram-Login account, graph.facebook.com for a Facebook-Login one).
// Accounts come from the P1.10 fixture extension.

import { beforeAll, describe, expect, it } from "vitest"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import { unordered } from "../../src/normalize.ts"
import { uniqueHost, uniqueId } from "../../src/unique.ts"
import {
  type Api,
  connectInstagram,
  createPublishedFlow,
  expectError,
  expectInvalid,
  expectNotFound,
  graphPath,
  type IgAccount,
  newTenant,
  parseBody,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueName,
  upstreamCalls,
} from "./helpers.ts"

const IG_HOST = "graph.instagram.com"
const FB_HOST = "graph.facebook.com"
const PROFILE = new RegExp(graphPath("me/messenger_profile"))

const SUMMARY_KEYS = [
  "adsEligible",
  "capiConnected",
  "capiDisconnected",
  "capiTestEventCode",
  "channel",
  "coexistEnabled",
  "datasetId",
  "displayPhoneNumber",
  "externalId",
  "handoverResumeFlowId",
  "hasCapiScope",
  "id",
  "inboxId",
  "isCoexist",
  "name",
  "syncTagEnabledAt",
  "tokenRefreshError",
  "username",
  "wabaId",
]

const summaryOf = (account: IgAccount, extra: Record<string, unknown> = {}) => ({
  id: account.integrationId,
  channel: "instagram",
  inboxId: account.inboxId,
  name: account.name,
  externalId: account.igId,
  wabaId: null,
  displayPhoneNumber: null,
  username: account.username,
  coexistEnabled: false,
  isCoexist: null,
  hasCapiScope: false,
  capiConnected: false,
  datasetId: null,
  capiTestEventCode: null,
  capiDisconnected: false,
  syncTagEnabledAt: null,
  handoverResumeFlowId: null,
  tokenRefreshError: null,
  adsEligible: false,
  ...extra,
})

type Settings = {
  welcomeFlowId: string | null
  conversationStarters: Array<{ question: string; flowId: string }>
  persistentMenus: Array<{ type: string; label: string; flowId?: string; url?: string }>
}

beforeAll(async () => {
  // Every empty-list write deletes profile fields; the body carries no
  // account data, so one unlimited answer serves every test of this file.
  await fake.script({
    match: { host: IG_HOST, method: "DELETE", pathRegex: graphPath("me/messenger_profile") },
    respond: { status: 200, json: { result: "success" } },
  })
})

/** Answers the profile POST whose body holds `marker` (unique per test). */
const answerProfilePost = (host: string, marker: string, status = 200) =>
  fake.script({
    match: { host, method: "POST", pathRegex: graphPath("me/messenger_profile"), bodyIncludes: marker },
    respond:
      status === 200
        ? { status, json: { result: "success" } }
        : { status, json: { error: { message: "(#100) Invalid parameter", type: "OAuthException", code: 100 } } },
  })

/** Profile calls of one account (matched by its bearer token). */
const profileCalls = async (account: IgAccount, since: number, host = IG_HOST): Promise<UpstreamRecord[]> =>
  (await upstreamCalls({ host, since, path: PROFILE })).filter(
    (record) => record.headers.authorization === `Bearer ig-token-${account.igId}`,
  )

const getSettings = async (api: Api, account: IgAccount): Promise<Settings> => {
  const response = await api.get<Settings>(`/v1/instagram-channels/${account.integrationId}/settings`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("instagramChannels.list / get", () => {
  it("lists both login types with ids and CAPI state, never credentials", async () => {
    const { tenant, api } = await newTenant("p110-ig-list")
    const native = await connectInstagram(tenant, "instagram")
    const viaFacebook = await connectInstagram(tenant, "facebook", {
      hasCapiScope: true,
      datasetId: "ds-1",
      capiTestEventCode: "TEST1",
      tokenRefreshError: "Token expired",
    })
    const response = await api.get<Array<Record<string, unknown>>>("/v1/instagram-channels")
    expect(response.status, response.text).toBe(200)
    for (const item of response.json) {
      expect(Object.keys(item).sort()).toEqual(SUMMARY_KEYS)
    }
    expect(unordered(response.json, (item) => String(item.id))).toEqual(
      unordered(
        [
          summaryOf(native),
          summaryOf(viaFacebook, {
            adsEligible: true,
            hasCapiScope: true,
            capiConnected: true,
            datasetId: "ds-1",
            capiTestEventCode: "TEST1",
            tokenRefreshError: "Token expired",
          }),
        ],
        (item) => String(item.id),
      ),
    )
    expect(response.text).not.toContain("ig-token-")
    expect(response.text).not.toContain("parity-secret")
  })

  it("capiConnected: a dataset without the scope reads false; a user disconnect wins", async () => {
    const { tenant, api } = await newTenant("p110-ig-capi-state")
    const lapsed = await connectInstagram(tenant, "facebook", { datasetId: "ds-lapsed" })
    const disconnected = await connectInstagram(tenant, "facebook", {
      datasetId: "ds-off",
      hasCapiScope: true,
      capiDisconnected: true,
    })
    const one = await api.get(`/v1/instagram-channels/${lapsed.integrationId}`)
    expect(one.json).toMatchObject({ datasetId: "ds-lapsed", hasCapiScope: false, capiConnected: false, capiDisconnected: false })
    const two = await api.get(`/v1/instagram-channels/${disconnected.integrationId}`)
    expect(two.json).toMatchObject({ datasetId: "ds-off", hasCapiScope: true, capiConnected: false, capiDisconnected: true })
  })

  it("get returns one account; another workspace's or an unknown id is 404", async () => {
    const { tenant, api } = await newTenant("p110-ig-get")
    const account = await connectInstagram(tenant, "instagram")
    const response = await api.get(`/v1/instagram-channels/${account.integrationId}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual(summaryOf(account))
    expect(String(account.integrationId)).toMatch(SNOWFLAKE)

    const { api: otherApi } = await newTenant("p110-ig-get-o")
    expectNotFound(await otherApi.get(`/v1/instagram-channels/${account.integrationId}`), "Channel integration not found")
    expectNotFound(await api.get(`/v1/instagram-channels/${UNKNOWN_ID}`), "Channel integration not found")
    expectInvalid(await api.get("/v1/instagram-channels/abc"), "id")
    expect((await otherApi.get("/v1/instagram-channels")).json).toEqual([])
  })
})

describe("instagramChannels.getSettings", () => {
  it("is empty for a new account; 404 for an unknown or foreign id", async () => {
    const { tenant, api } = await newTenant("p110-ig-settings-get")
    const account = await connectInstagram(tenant, "instagram")
    expect(await getSettings(api, account)).toEqual({ welcomeFlowId: null, conversationStarters: [], persistentMenus: [] })
    expectNotFound(await api.get(`/v1/instagram-channels/${UNKNOWN_ID}/settings`), "Record not found")
    const { api: otherApi } = await newTenant("p110-ig-settings-get-o")
    expectNotFound(await otherApi.get(`/v1/instagram-channels/${account.integrationId}/settings`), "Record not found")
    expectInvalid(await api.get("/v1/instagram-channels/abc/settings"), "id")
  })
})

describe("instagramChannels.updateSettings (PUT)", () => {
  it("saves every field, pushes ice breakers + menu to Instagram, and sets markReadOnOutbound", async () => {
    const { tenant, api } = await newTenant("p110-ig-put")
    const account = await connectInstagram(tenant, "instagram")
    const welcome = await createPublishedFlow(api, uniqueName("Welcome"), "Hi")
    const starterFlow = await createPublishedFlow(api, uniqueName("Starter"), "Starter")
    const marker = `Question ${uniqueId()}`
    const menuUrl = `https://${uniqueHost("p110-menu")}/menu`
    await answerProfilePost(IG_HOST, marker)
    const since = await fake.cursor()
    const settings: Settings = {
      welcomeFlowId: welcome,
      conversationStarters: [{ question: marker, flowId: starterFlow }],
      persistentMenus: [
        { type: "flow", label: "Start", flowId: welcome },
        { type: "url", label: "Website", url: menuUrl },
      ],
    }
    const response = await api.put(`/v1/instagram-channels/${account.integrationId}/settings`, {
      ...settings,
      markReadOnOutbound: true,
    })
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expect(await getSettings(api, account)).toEqual(settings)

    const calls = await profileCalls(account, since)
    expect(calls.map((call) => call.method)).toEqual(["POST"])
    const [post] = calls as [UpstreamRecord]
    expect(post.query).toMatchObject({ platform: "instagram", access_token: `ig-token-${account.igId}` })
    const body = parseBody<{
      platform: string
      ice_breakers: Array<{ locale: string; call_to_actions: Array<{ question: string; payload: string }> }>
      persistent_menu: Array<{
        locale: string
        call_to_actions: Array<{ type: string; title: string; payload?: string; url?: string }>
      }>
    }>(post)
    expect(body.platform).toBe("instagram")
    expect(body.ice_breakers).toEqual([
      { locale: "default", call_to_actions: [{ question: marker, payload: expect.any(String) }] },
    ])
    expect(body.persistent_menu).toEqual([
      {
        locale: "default",
        call_to_actions: [
          { type: "postback", title: "Start", payload: expect.any(String) },
          { type: "web_url", title: "Website", url: menuUrl },
        ],
      },
    ])
    expect(body.ice_breakers[0]?.call_to_actions[0]?.payload.length).toBeGreaterThan(0)

    const inboxes = await api.get<{ data: Array<{ id: string; markReadOnOutbound: boolean }> }>("/v1/inboxes")
    expect(inboxes.json.data.find((inbox) => inbox.id === account.inboxId)?.markReadOnOutbound).toBe(true)
  })

  it("empty lists delete both profile fields and send no profile update", async () => {
    const { tenant, api } = await newTenant("p110-ig-put-empty")
    const account = await connectInstagram(tenant, "instagram")
    const since = await fake.cursor()
    const response = await api.put(`/v1/instagram-channels/${account.integrationId}/settings`, {
      welcomeFlowId: null,
      conversationStarters: [],
      persistentMenus: [],
    })
    expect(response.status, response.text).toBe(204)
    const calls = await profileCalls(account, since)
    expect(calls.map((call) => call.method)).toEqual(["DELETE"])
    expect(parseBody(calls[0] as UpstreamRecord)).toEqual({ platform: "instagram", fields: ["ICE_BREAKERS", "PERSISTENT_MENU"] })
    // markReadOnOutbound omitted: unchanged.
    const inboxes = await api.get<{ data: Array<{ id: string; markReadOnOutbound: boolean }> }>("/v1/inboxes")
    expect(inboxes.json.data.find((inbox) => inbox.id === account.inboxId)?.markReadOnOutbound).toBe(false)
  })

  it("only ice breakers: deletes the menu field, then posts the ice breakers", async () => {
    const { tenant, api } = await newTenant("p110-ig-put-starters")
    const account = await connectInstagram(tenant, "instagram")
    const flow = await createPublishedFlow(api, uniqueName("Starter"), "Starter")
    const marker = `Only ${uniqueId()}`
    await answerProfilePost(IG_HOST, marker)
    const since = await fake.cursor()
    const response = await api.put(`/v1/instagram-channels/${account.integrationId}/settings`, {
      welcomeFlowId: null,
      conversationStarters: [{ question: marker, flowId: flow }],
      persistentMenus: [],
    })
    expect(response.status, response.text).toBe(204)
    const calls = await profileCalls(account, since)
    expect(calls.map((call) => call.method)).toEqual(["DELETE", "POST"])
    expect(parseBody(calls[0] as UpstreamRecord)).toEqual({ platform: "instagram", fields: ["PERSISTENT_MENU"] })
    const posted = parseBody<Record<string, unknown>>(calls[1] as UpstreamRecord)
    expect(Object.keys(posted).sort()).toEqual(["ice_breakers", "platform"])
  })

  it("a Facebook-Login account pushes to graph.facebook.com", async () => {
    const { tenant, api } = await newTenant("p110-ig-put-fb")
    const account = await connectInstagram(tenant, "facebook")
    const flow = await createPublishedFlow(api, uniqueName("Starter"), "Starter")
    const marker = `Fb ${uniqueId()}`
    await answerProfilePost(FB_HOST, marker)
    const since = await fake.cursor()
    const response = await api.put(`/v1/instagram-channels/${account.integrationId}/settings`, {
      welcomeFlowId: null,
      conversationStarters: [{ question: marker, flowId: flow }],
      persistentMenus: [{ type: "url", label: "Site", url: `https://${uniqueHost("p110-fbmenu")}/x` }],
    })
    expect(response.status, response.text).toBe(204)
    expect((await profileCalls(account, since, FB_HOST)).map((call) => call.method)).toEqual(["POST"])
    expect(await profileCalls(account, since, IG_HOST)).toEqual([])
  })

  it("an Instagram error fails the write (400 systemError) and keeps the saved settings", async () => {
    const { tenant, api } = await newTenant("p110-ig-put-fail")
    const account = await connectInstagram(tenant, "instagram")
    const flow = await createPublishedFlow(api, uniqueName("Starter"), "Starter")
    const marker = `Fail ${uniqueId()}`
    await answerProfilePost(IG_HOST, marker, 400)
    const response = await api.put(`/v1/instagram-channels/${account.integrationId}/settings`, {
      welcomeFlowId: flow,
      conversationStarters: [{ question: marker, flowId: flow }],
      persistentMenus: [{ type: "flow", label: "Go", flowId: flow }],
    })
    expectError(response, 400, "systemError", { defined: true, message: "Failed to update Instagram integration" })
    expect(await getSettings(api, account)).toEqual({ welcomeFlowId: null, conversationStarters: [], persistentMenus: [] })
  })

  it("a flow of another workspace is 404 notFound before any Instagram call", async () => {
    const { tenant, api } = await newTenant("p110-ig-put-flow")
    const account = await connectInstagram(tenant, "instagram")
    const { api: otherApi } = await newTenant("p110-ig-put-flow-o")
    const foreignFlow = await createPublishedFlow(otherApi, uniqueName("Foreign"), "x")
    const since = await fake.cursor()
    expectNotFound(
      await api.put(`/v1/instagram-channels/${account.integrationId}/settings`, {
        welcomeFlowId: foreignFlow,
        conversationStarters: [],
        persistentMenus: [],
      }),
      "Flow does not exists.",
    )
    expect(await profileCalls(account, since)).toEqual([])
  })

  it("an unknown account id answers 400 systemError (the lookup runs inside the write's catch-all)", async () => {
    const { api } = await newTenant("p110-ig-put-404")
    expectError(
      await api.put(`/v1/instagram-channels/${UNKNOWN_ID}/settings`, {
        welcomeFlowId: null,
        conversationStarters: [],
        persistentMenus: [],
      }),
      400,
      "systemError",
      { message: "Failed to update Instagram integration" },
    )
  })

  it("validates the body (422 invalidRequestData)", async () => {
    const { tenant, api } = await newTenant("p110-ig-put-invalid")
    const account = await connectInstagram(tenant, "instagram")
    const path = `/v1/instagram-channels/${account.integrationId}/settings`
    expectInvalid(await api.put(path, { conversationStarters: [], persistentMenus: [] }), "welcomeFlowId")
    expectInvalid(await api.put(path, { welcomeFlowId: null, persistentMenus: [] }), "conversationStarters")
    expectInvalid(
      await api.put(path, {
        welcomeFlowId: null,
        conversationStarters: [],
        persistentMenus: [{ type: "url", label: "Bad", url: "not a url" }],
      }),
      "persistentMenus.0.url",
    )
    expectInvalid(
      await api.put(path, {
        welcomeFlowId: null,
        conversationStarters: [],
        persistentMenus: [{ type: "url", label: "", url: "https://example.parity.test" }],
      }),
      "persistentMenus.0.label",
    )
    expectInvalid(
      await api.put(path, { welcomeFlowId: null, conversationStarters: [{ question: "q" }], persistentMenus: [] }),
      "conversationStarters.0.flowId",
    )
  })
})

describe("instagramChannels.patchSettings (PATCH)", () => {
  it("changes only the sent fields, keeps the others, and pushes the merged settings again", async () => {
    const { tenant, api } = await newTenant("p110-ig-patch")
    const account = await connectInstagram(tenant, "instagram")
    const first = await createPublishedFlow(api, uniqueName("First"), "1")
    const second = await createPublishedFlow(api, uniqueName("Second"), "2")
    const marker = `Patch ${uniqueId()}`
    await answerProfilePost(IG_HOST, marker)
    const saved: Settings = {
      welcomeFlowId: first,
      conversationStarters: [{ question: marker, flowId: first }],
      persistentMenus: [{ type: "flow", label: "Menu", flowId: first }],
    }
    expect((await api.put(`/v1/instagram-channels/${account.integrationId}/settings`, saved)).status).toBe(204)

    const since = await fake.cursor()
    const response = await api.patch(`/v1/instagram-channels/${account.integrationId}/settings`, { welcomeFlowId: second })
    expect(response.status, response.text).toBe(204)
    expect(await getSettings(api, account)).toEqual({ ...saved, welcomeFlowId: second })
    // The saved ice breakers and menu are pushed again.
    const calls = await profileCalls(account, since)
    expect(calls.map((call) => call.method)).toEqual(["POST"])
    expect(calls[0]?.bodyText).toContain(marker)

    // welcomeFlowId: null clears it; markReadOnOutbound alone sets the inbox flag.
    expect((await api.patch(`/v1/instagram-channels/${account.integrationId}/settings`, { welcomeFlowId: null })).status).toBe(204)
    expect((await getSettings(api, account)).welcomeFlowId).toBeNull()
    expect(
      (await api.patch(`/v1/instagram-channels/${account.integrationId}/settings`, { markReadOnOutbound: true })).status,
    ).toBe(204)
    const inboxes = await api.get<{ data: Array<{ id: string; markReadOnOutbound: boolean }> }>("/v1/inboxes")
    expect(inboxes.json.data.find((inbox) => inbox.id === account.inboxId)?.markReadOnOutbound).toBe(true)
    expect(await getSettings(api, account)).toEqual({ ...saved, welcomeFlowId: null })
  })

  it("an empty body re-pushes the saved (empty) settings", async () => {
    const { tenant, api } = await newTenant("p110-ig-patch-empty")
    const account = await connectInstagram(tenant, "instagram")
    const since = await fake.cursor()
    const response = await api.patch(`/v1/instagram-channels/${account.integrationId}/settings`, {})
    expect(response.status, response.text).toBe(204)
    const calls = await profileCalls(account, since)
    expect(calls.map((call) => call.method)).toEqual(["DELETE"])
  })

  it("404 notFound for an unknown or foreign account; 422 for bad input", async () => {
    const { tenant, api } = await newTenant("p110-ig-patch-404")
    const account = await connectInstagram(tenant, "instagram")
    expectNotFound(await api.patch(`/v1/instagram-channels/${UNKNOWN_ID}/settings`, { welcomeFlowId: null }), "Record not found")
    const { api: otherApi } = await newTenant("p110-ig-patch-404-o")
    expectNotFound(
      await otherApi.patch(`/v1/instagram-channels/${account.integrationId}/settings`, { welcomeFlowId: null }),
      "Record not found",
    )
    expectInvalid(
      await api.patch(`/v1/instagram-channels/${account.integrationId}/settings`, { persistentMenus: [{ type: "phone" }] }),
      "persistentMenus.0.type",
    )
    expectInvalid(
      await api.patch(`/v1/instagram-channels/${account.integrationId}/settings`, { markReadOnOutbound: 1 }),
      "markReadOnOutbound",
    )
  })
})
