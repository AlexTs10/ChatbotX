// messengerChannels.list / get (channel summaries, no credentials),
// getSettings / updateSettings (PUT) / patchSettings (PATCH), and
// messengerPersonas.list (scope `channels`).
//
// A settings write saves welcome flow, persistent menu, personas and ice
// breakers, then pushes them to Facebook with the page token:
//   personas  -> `POST /me/personas?access_token=` for each persona without a
//                Facebook id, `GET /me/personas`, `DELETE /<persona id>` for
//                each Facebook persona no longer used;
//   profile   -> `DELETE /v23.0/me/messenger_profile` {fields} for an empty
//                menu / ice-breaker list, `POST /v23.0/me/messenger_profile`
//                {get_started, persistent_menu?, ice_breakers?}, and once per
//                page per 5 minutes a read-back
//                `GET /v23.0/me/messenger_profile?fields=get_started,ice_breakers,greeting`.
// The app URL is http, so no whitelisted-domain call is made.
// `/me/...` paths are shared by every page: the answers that carry no page
// data are unlimited scripts set once in this file (calls are told apart by
// the page token); answers with page data match a unique body marker or are
// consumed in the same test.

import { beforeAll, describe, expect, it } from "vitest"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import { unordered } from "../../src/normalize.ts"
import { uniqueHost, uniqueId } from "../../src/unique.ts"
import {
  type Api,
  connectMessenger,
  createPublishedFlow,
  expectInvalid,
  expectNotFound,
  expectSystemError,
  GRAPH,
  graphPath,
  keysOf,
  type MessengerPage,
  newTenant,
  parseBody,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueDigits,
  uniqueName,
  upstreamCalls,
} from "./helpers.ts"

const PROFILE = new RegExp(graphPath("me/messenger_profile"))
const PERSONAS = /^\/me\/personas$/

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

const summaryOf = (page: MessengerPage, extra: Record<string, unknown> = {}) => ({
  id: page.integrationId,
  channel: "messenger",
  inboxId: page.inboxId,
  name: page.name,
  externalId: page.pageId,
  wabaId: null,
  displayPhoneNumber: null,
  username: null,
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
  adsEligible: true,
  ...extra,
})

type Persona = {
  id: string
  name: string
  isDefault: boolean
  facebookPersonaId?: string
  profilePicture: { id: string; url: string; mode: string }
}

type Settings = {
  welcomeFlowId: string | null
  persistentMenus: Array<{ type: string; label: string; flowId?: string; url?: string }>
  personas: Persona[]
  conversationStarters: Array<{ question: string; flowId: string }>
}

const EMPTY: Settings = { welcomeFlowId: null, persistentMenus: [], personas: [], conversationStarters: [] }

beforeAll(async () => {
  // Answers without page data, shared by every page of this file.
  await fake.script(
    {
      match: { host: GRAPH, method: "DELETE", pathRegex: graphPath("me/messenger_profile") },
      respond: { status: 200, json: { result: "success" } },
    },
    {
      match: { host: GRAPH, method: "POST", pathRegex: graphPath("me/messenger_profile") },
      respond: { status: 200, json: { result: "success" } },
    },
    {
      match: { host: GRAPH, method: "GET", pathRegex: graphPath("me/messenger_profile") },
      respond: { status: 200, json: { data: [{}] } },
    },
    {
      match: { host: GRAPH, method: "GET", path: "/me/personas" },
      respond: { status: 200, json: { data: [] } },
    },
  )
})

const settingsPath = (page: MessengerPage) => `/v1/messenger-channels/${page.integrationId}/settings`

const getSettings = async (api: Api, page: MessengerPage): Promise<Settings> => {
  const response = await api.get<Settings>(settingsPath(page))
  expect(response.status, response.text).toBe(200)
  return response.json
}

/**
 * Graph calls made with this page's token since `since` (persona creates carry
 * it as `access_token`). The welcome-message-flow mirror of a just-published
 * flow (an async worker job of `flows.publish`, not of these routes) is left out.
 */
const pageCalls = async (page: MessengerPage, since: number): Promise<UpstreamRecord[]> =>
  (await upstreamCalls({ host: GRAPH, since })).filter(
    (record) =>
      (record.headers.authorization === `Bearer ${page.accessToken}` || record.query.access_token === page.accessToken) &&
      !record.path.endsWith("/welcome_message_flows"),
  )

const callSummary = (records: UpstreamRecord[]) =>
  records.map((record) => `${record.method} ${record.path.replace(/^\/v[0-9.]+\//, "/v/")}`)

describe("messengerChannels.list / get", () => {
  it("lists the workspace's pages with ids and CAPI state, never credentials", async () => {
    const { tenant, api } = await newTenant("p111-mc-list")
    const plain = await connectMessenger(tenant)
    const capi = await connectMessenger(tenant, {
      hasCapiScope: true,
      datasetId: "ds-1",
      capiTestEventCode: "TEST1",
      tokenRefreshError: "Token expired",
    })
    const response = await api.get<Array<Record<string, unknown>>>("/v1/messenger-channels")
    expect(response.status, response.text).toBe(200)
    for (const item of response.json) {
      expect(keysOf(item)).toEqual(SUMMARY_KEYS)
    }
    expect(unordered(response.json, (item) => String(item.id))).toEqual(
      unordered(
        [
          summaryOf(plain),
          summaryOf(capi, {
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
    expect(response.text).not.toContain("page-token-")
    expect(response.text).not.toContain("parity-secret")
  })

  it("get returns one page; another workspace's, an unknown or a non-numeric id is refused", async () => {
    const { tenant, api } = await newTenant("p111-mc-get")
    const page = await connectMessenger(tenant, { datasetId: "ds-lapsed" })
    const response = await api.get(`/v1/messenger-channels/${page.integrationId}`)
    expect(response.status, response.text).toBe(200)
    // A dataset without the CAPI scope is not connected.
    expect(response.json).toEqual(summaryOf(page, { datasetId: "ds-lapsed" }))
    expect(page.integrationId).toMatch(SNOWFLAKE)
    const { api: otherApi } = await newTenant("p111-mc-get-o")
    expectNotFound(await otherApi.get(`/v1/messenger-channels/${page.integrationId}`), "Channel integration not found")
    expectNotFound(await api.get(`/v1/messenger-channels/${UNKNOWN_ID}`), "Channel integration not found")
    expectInvalid(await api.get("/v1/messenger-channels/abc"), "id")
    expect((await otherApi.get("/v1/messenger-channels")).json).toEqual([])
  })
})

describe("messengerChannels.getSettings", () => {
  it("is empty for a new page; 404 for an unknown or foreign id", async () => {
    const { tenant, api } = await newTenant("p111-mc-settings-get")
    const page = await connectMessenger(tenant)
    expect(await getSettings(api, page)).toEqual(EMPTY)
    expectNotFound(await api.get(`/v1/messenger-channels/${UNKNOWN_ID}/settings`), "Record not found")
    const { api: otherApi } = await newTenant("p111-mc-settings-get-o")
    expectNotFound(await otherApi.get(settingsPath(page)), "Record not found")
    expectInvalid(await api.get("/v1/messenger-channels/abc/settings"), "id")
  })
})

describe("messengerChannels.updateSettings (PUT)", () => {
  it("saves every field, registers the persona and pushes menu + ice breakers to Facebook", async () => {
    const { tenant, api } = await newTenant("p111-mc-put")
    const page = await connectMessenger(tenant)
    const welcome = await createPublishedFlow(api, uniqueName("Welcome"), "Hi")
    const starterFlow = await createPublishedFlow(api, uniqueName("Starter"), "Starter")
    const question = `Question ${uniqueId()}`
    const personaName = `Agent ${uniqueId()}`
    const facebookPersonaId = uniqueDigits()
    const pictureUrl = `https://${uniqueHost("p111-pic")}/a.png`
    const menuUrl = `https://${uniqueHost("p111-menu")}/menu`
    await fake.script({
      match: { host: GRAPH, method: "POST", path: "/me/personas", bodyIncludes: personaName },
      respond: { status: 200, json: { id: facebookPersonaId } },
    })
    const since = await fake.cursor()
    const response = await api.put(settingsPath(page), {
      welcomeFlowId: welcome,
      persistentMenus: [
        { type: "flow", label: "Start", flowId: welcome },
        { type: "url", label: "Website", url: menuUrl },
      ],
      personas: [{ name: personaName, isDefault: true, profilePictureUrl: pictureUrl }],
      conversationStarters: [{ question, flowId: starterFlow }],
      markReadOnOutbound: true,
    })
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")

    const saved = await getSettings(api, page)
    expect(saved).toEqual({
      welcomeFlowId: welcome,
      persistentMenus: [
        { type: "flow", label: "Start", flowId: welcome },
        { type: "url", label: "Website", url: menuUrl },
      ],
      personas: [
        {
          id: expect.stringMatching(SNOWFLAKE),
          name: personaName,
          isDefault: true,
          facebookPersonaId,
          profilePicture: { id: expect.stringMatching(SNOWFLAKE), url: pictureUrl, mode: "url" },
        },
      ],
      conversationStarters: [{ question, flowId: starterFlow }],
    })

    const calls = await pageCalls(page, since)
    expect(callSummary(calls)).toEqual([
      "POST /me/personas",
      "GET /me/personas",
      "POST /v/me/messenger_profile",
      "GET /v/me/messenger_profile",
    ])
    const [create, , profile, readBack] = calls as [UpstreamRecord, UpstreamRecord, UpstreamRecord, UpstreamRecord]
    expect(parseBody(create)).toEqual({ name: personaName, profile_picture_url: pictureUrl })
    expect(readBack.query).toEqual({ fields: "get_started,ice_breakers,greeting" })
    const body = parseBody<{
      get_started: { payload: string }
      persistent_menu: Array<{
        locale: string
        composer_input_disabled: boolean
        call_to_actions: Array<{ type: string; title: string; payload?: string; url?: string }>
      }>
      ice_breakers: Array<{ question: string; payload: string }>
    }>(profile)
    expect(keysOf(body)).toEqual(["get_started", "ice_breakers", "persistent_menu"])
    expect(body.get_started.payload).toEqual(expect.any(String))
    expect(body.get_started.payload).not.toBe("GET_STARTED")
    expect(body.ice_breakers).toEqual([{ question, payload: expect.any(String) }])
    expect(body.persistent_menu).toEqual([
      {
        locale: "default",
        composer_input_disabled: false,
        call_to_actions: [
          { type: "postback", title: "Start", payload: expect.any(String) },
          { type: "web_url", title: "Website", url: menuUrl },
        ],
      },
    ])

    // The default persona's Facebook id is the page persona; the persona list shows it.
    const personas = await api.get<{ data: Array<{ id: string; name: string; pageName: string }> }>(
      "/v1/messenger-personas",
    )
    expect(personas.status, personas.text).toBe(200)
    expect(personas.json).toEqual({ data: [{ id: saved.personas[0]?.id, name: personaName, pageName: page.name }] })

    const inboxes = await api.get<{ data: Array<{ id: string; markReadOnOutbound: boolean }> }>("/v1/inboxes")
    expect(inboxes.json.data.find((inbox) => inbox.id === page.inboxId)?.markReadOnOutbound).toBe(true)
  })

  it("empty lists delete both profile fields; a save without a welcome flow sends GET_STARTED", async () => {
    const { tenant, api } = await newTenant("p111-mc-put-empty")
    const page = await connectMessenger(tenant)
    const since = await fake.cursor()
    const response = await api.put(settingsPath(page), { ...EMPTY })
    expect(response.status, response.text).toBe(204)
    const calls = await pageCalls(page, since)
    expect(callSummary(calls)).toEqual([
      "GET /me/personas",
      "DELETE /v/me/messenger_profile",
      "POST /v/me/messenger_profile",
      "GET /v/me/messenger_profile",
    ])
    expect(parseBody(calls[1] as UpstreamRecord)).toEqual({ fields: ["PERSISTENT_MENU", "ICE_BREAKERS"] })
    expect(parseBody(calls[2] as UpstreamRecord)).toEqual({ get_started: { payload: "GET_STARTED" } })
    expect(await getSettings(api, page)).toEqual(EMPTY)

    // A second save within 5 minutes skips the read-back.
    const again = await fake.cursor()
    expect((await api.put(settingsPath(page), { ...EMPTY })).status).toBe(204)
    expect(callSummary(await pageCalls(page, again))).toEqual([
      "GET /me/personas",
      "DELETE /v/me/messenger_profile",
      "POST /v/me/messenger_profile",
    ])
  })

  it("a persona left out is deleted from Facebook; an unchanged one keeps its Facebook id", async () => {
    const { tenant, api } = await newTenant("p111-mc-personas")
    const keptFb = uniqueDigits()
    const staleFb = uniqueDigits()
    const kept: Persona = {
      id: uniqueDigits(),
      name: "Kept",
      isDefault: false,
      facebookPersonaId: keptFb,
      profilePicture: { id: uniqueDigits(), url: "https://parity.test/kept.png", mode: "url" },
    }
    const stale: Persona = {
      id: uniqueDigits(),
      name: "Stale",
      isDefault: false,
      facebookPersonaId: staleFb,
      profilePicture: { id: uniqueDigits(), url: "https://parity.test/stale.png", mode: "url" },
    }
    const page = await connectMessenger(tenant, { personas: [kept, stale] })
    // This save's own persona list read (consumed below); then the stale persona's delete.
    await fake.script(
      {
        match: { host: GRAPH, method: "GET", path: "/me/personas" },
        respond: { status: 200, json: { data: [{ id: keptFb }, { id: staleFb }] } },
        times: 1,
      },
      { match: { host: GRAPH, method: "DELETE", path: `/${staleFb}` }, respond: { status: 200, json: { success: true } } },
    )
    const since = await fake.cursor()
    const response = await api.patch(settingsPath(page), {
      personas: [{ id: kept.id, name: kept.name, isDefault: false, profilePicture: kept.profilePicture }],
    })
    expect(response.status, response.text).toBe(204)
    const calls = await pageCalls(page, since)
    expect(callSummary(calls).filter((call) => !call.includes("messenger_profile"))).toEqual([
      "GET /me/personas",
      `DELETE /${staleFb}`,
    ])
    const saved = await getSettings(api, page)
    expect(saved.personas).toEqual([kept])
    // Only registered personas are offered.
    const personas = await api.get<{ data: Array<{ id: string }> }>("/v1/messenger-personas")
    expect(personas.json.data).toEqual([{ id: kept.id, name: "Kept", pageName: page.name }])
  })

  it("validates flows, personas and menus (404 / 422); nothing is sent to Facebook", async () => {
    const { tenant, api } = await newTenant("p111-mc-put-invalid")
    const page = await connectMessenger(tenant)
    const since = await fake.cursor()
    expectNotFound(await api.put(settingsPath(page), { ...EMPTY, welcomeFlowId: UNKNOWN_ID }), "Flow does not exists.")
    expectInvalid(
      await api.put(settingsPath(page), {
        ...EMPTY,
        personas: [
          { name: "A", isDefault: true, profilePictureUrl: "https://parity.test/a.png" },
          { name: "B", isDefault: true, profilePictureUrl: "https://parity.test/b.png" },
        ],
      }),
      "personas",
    )
    expectInvalid(await api.put(settingsPath(page), { ...EMPTY, personas: [{ name: "A", isDefault: false }] }), "personas.0.profilePictureUrl")
    expectInvalid(
      await api.put(settingsPath(page), { ...EMPTY, persistentMenus: [{ type: "url", label: "X", url: "not-a-url" }] }),
      "persistentMenus.0.url",
    )
    expectInvalid(await api.put(settingsPath(page), { welcomeFlowId: null }), "persistentMenus")
    expect(await pageCalls(page, since)).toEqual([])
    expect(await getSettings(api, page)).toEqual(EMPTY)
    // PUT on an unknown page is a 400 (the lookup runs inside the save's catch-all); PATCH and GET answer 404.
    expectSystemError(await api.put(`/v1/messenger-channels/${UNKNOWN_ID}/settings`, { ...EMPTY }), "Failed to update Facebook page")
  })

  it("a persona Facebook refuses as default fails the save (400) and keeps the old settings", async () => {
    const { tenant, api } = await newTenant("p111-mc-persona-fail")
    const page = await connectMessenger(tenant)
    const personaName = `Refused ${uniqueId()}`
    await fake.script({
      match: { host: GRAPH, method: "POST", path: "/me/personas", bodyIncludes: personaName },
      respond: { status: 400, json: { error: { message: "(#100) Invalid image", type: "OAuthException", code: 100 } } },
    })
    const response = await api.put(settingsPath(page), {
      ...EMPTY,
      personas: [{ name: personaName, isDefault: true, profilePictureUrl: "https://parity.test/x.png" }],
    })
    expectSystemError(response, "Couldn't register the default persona with Facebook. Please try saving again.")
    expect(await getSettings(api, page)).toEqual(EMPTY)
  })
})

describe("messengerChannels.patchSettings (PATCH)", () => {
  it("changes only the given fields and pushes the merged profile", async () => {
    const { tenant, api } = await newTenant("p111-mc-patch")
    const page = await connectMessenger(tenant)
    const flow = await createPublishedFlow(api, uniqueName("Menu flow"), "Menu")
    expect((await api.put(settingsPath(page), { ...EMPTY, persistentMenus: [{ type: "flow", label: "Go", flowId: flow }] })).status).toBe(204)
    const question = `Patched ${uniqueId()}`
    const since = await fake.cursor()
    const response = await api.patch(settingsPath(page), { conversationStarters: [{ question, flowId: flow }] })
    expect(response.status, response.text).toBe(204)
    expect(await getSettings(api, page)).toEqual({
      ...EMPTY,
      persistentMenus: [{ type: "flow", label: "Go", flowId: flow }],
      conversationStarters: [{ question, flowId: flow }],
    })
    const calls = await pageCalls(page, since)
    expect(callSummary(calls)).toEqual(["GET /me/personas", "POST /v/me/messenger_profile"])
    const body = parseBody<Record<string, unknown>>(calls[1] as UpstreamRecord)
    expect(keysOf(body)).toEqual(["get_started", "ice_breakers", "persistent_menu"])

    // An empty patch re-pushes the saved settings unchanged.
    expect((await api.patch(settingsPath(page), {})).status).toBe(204)
    expect((await getSettings(api, page)).conversationStarters).toEqual([{ question, flowId: flow }])
    expectNotFound(await api.patch(`/v1/messenger-channels/${UNKNOWN_ID}/settings`, {}), "Record not found")
    expectInvalid(await api.patch(settingsPath(page), { conversationStarters: [{ question: "x" }] }), "conversationStarters.0.flowId")
  })
})

describe("messengerPersonas.list", () => {
  it("lists registered personas of every page of the workspace, never another workspace's", async () => {
    const { tenant, api } = await newTenant("p111-personas")
    const registered = (name: string): Persona => ({
      id: uniqueDigits(),
      name,
      isDefault: false,
      facebookPersonaId: uniqueDigits(),
      profilePicture: { id: uniqueDigits(), url: "https://parity.test/p.png", mode: "url" },
    })
    const a = registered("Anna")
    const b = registered("Ben")
    const unregistered: Persona = { ...registered("Draft"), facebookPersonaId: undefined }
    delete unregistered.facebookPersonaId
    const pageA = await connectMessenger(tenant, { personas: [a, unregistered] })
    const pageB = await connectMessenger(tenant, { personas: [b] })
    const response = await api.get<{ data: Array<{ id: string; name: string; pageName: string }> }>("/v1/messenger-personas")
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(["data"])
    expect(unordered(response.json.data, (item) => item.id)).toEqual(
      unordered(
        [
          { id: a.id, name: "Anna", pageName: pageA.name },
          { id: b.id, name: "Ben", pageName: pageB.name },
        ],
        (item) => item.id,
      ),
    )
    const { api: otherApi } = await newTenant("p111-personas-o")
    expect((await otherApi.get("/v1/messenger-personas")).json).toEqual({ data: [] })
  })
})
