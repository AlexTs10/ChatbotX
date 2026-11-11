// conversations.threadControl (scope `inbox`): take / release / pass / sync of
// a contact inbox's routing thread. WhatsApp calls
// `POST graph.facebook.com/v23.0/<phoneNumberId>/thread_control`, Messenger
// `POST /v23.0/me/take_thread_control | pass_thread_control` and
// `GET /v23.0/me/thread_owner` (all answered by the fake upstream). Effects:
// the returned snapshot, the stored routing columns (conversations.get), the
// realtime `contactInboxThreadControlUpdated` + `messageCreated` (divider)
// events, the divider message (contacts.listMessages), and the release job
// that archiving an owned WhatsApp thread enqueues.

import { randomInt } from "node:crypto"
import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import { P107_META_APP_ID } from "../../src/fixtures/ext/p1-07.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import { Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  contactPath,
  createWebchat,
  createWebchatContact,
  expectError,
  expectInvalid,
  ext,
  idOf,
  newTenant,
  UNKNOWN_ID,
} from "./helpers.ts"
import {
  type ConversationItem,
  getConversation,
  HOUR,
  listConversations,
  openWorkspaceSocket,
  seedConversation,
} from "./helpers-conversations.ts"

const GRAPH = "graph.facebook.com"
const BUSINESS_AI_APP_ID = "622851382610562"

type Snapshot = {
  contactInboxId: string
  threadControlState: string | null
  threadOwnerRole: string | null
  threadOwnerAppId: string | null
  threadControlUpdatedAt: string | null
  threadOwnerExpiresAt: string | null
  threadControlLastEvent: string | null
}
type ThreadControlResponse = { status: "applied" | "notEscalation"; snapshot?: Snapshot }

const digits = (length: number) => Array.from({ length }, () => randomInt(10)).join("")

const SNAPSHOT_KEYS = [
  "contactInboxId",
  "threadControlLastEvent",
  "threadControlState",
  "threadControlUpdatedAt",
  "threadOwnerAppId",
  "threadOwnerExpiresAt",
  "threadOwnerRole",
]

/** A WhatsApp channel + one contact (phone-keyed `sourceId`) with a conversation. */
const whatsappConversation = async (tenant: Tenant, contactInbox: Parameters<typeof seedConversation>[1] = {}) => {
  const phoneNumberId = `1${digits(14)}`
  const channel = await (await ext()).createWhatsappChannel({
    workspaceId: tenant.workspaceId,
    name: "Parity WhatsApp",
    phoneNumberId,
  })
  const waId = `1555${digits(7)}`
  const seeded = await seedConversation(tenant.workspaceId, {
    channel: "whatsapp",
    inboxId: channel.inboxId,
    firstName: "Wanda",
    phoneNumber: `+${waId}`,
    ...contactInbox,
    contactInbox: { sourceId: waId, ...(contactInbox.contactInbox ?? {}) },
  })
  return { ...seeded, phoneNumberId, waId, accessToken: channel.accessToken, path: `/v23.0/${phoneNumberId}/thread_control` }
}

/** A Messenger channel + one contact (PSID `sourceId`) with a conversation. */
const messengerConversation = async (tenant: Tenant, contactInbox: Parameters<typeof seedConversation>[1] = {}) => {
  const pageId = `9${digits(14)}`
  const channel = await (await ext()).createMessengerChannel({
    workspaceId: tenant.workspaceId,
    name: "Parity Page",
    pageId,
  })
  const psid = `7${digits(15)}`
  const seeded = await seedConversation(tenant.workspaceId, {
    channel: "messenger",
    inboxId: channel.inboxId,
    firstName: "Mona",
    ...contactInbox,
    contactInbox: { sourceId: psid, ...(contactInbox.contactInbox ?? {}) },
  })
  return { ...seeded, pageId, psid, accessToken: channel.accessToken }
}

const threadControl = (api: Api, conversationId: string, body: Record<string, unknown>) =>
  api.post<ThreadControlResponse>(`/v1/conversations/${conversationId}/thread-control`, body)

const recordsOf = (host: string, pathPrefix: string, filter: (r: UpstreamRecord) => boolean = () => true) =>
  fake.requests({ kind: "http", host, pathPrefix }).then((records) => records.filter(filter))

const bodyOf = (record: UpstreamRecord | undefined): unknown => JSON.parse(record?.bodyText ?? "null")

/** The stored routing columns of the conversation's only contact inbox. */
const storedRouting = async (api: Api, conversationId: string) => {
  const item: ConversationItem = await getConversation(api, conversationId)
  const inbox = item.contactInboxes[0]
  return {
    threadControlState: inbox?.threadControlState ?? null,
    threadOwnerRole: inbox?.threadOwnerRole ?? null,
    threadOwnerAppId: inbox?.threadOwnerAppId ?? null,
    threadControlUpdatedAt: inbox?.threadControlUpdatedAt ?? null,
    threadOwnerExpiresAt: inbox?.threadOwnerExpiresAt ?? null,
    threadControlLastEvent: inbox?.threadControlLastEvent ?? null,
  }
}

describe("threadControl on WhatsApp", () => {
  it("take: calls Meta with the note, returns the owned snapshot, stores it, writes a divider, sends realtime events", async () => {
    const n = new Normalizer()
    const { tenant, api } = await newTenant("p107-tc-wa-take")
    const wa = await whatsappConversation(tenant)
    await fake.script({
      match: { host: GRAPH, method: "POST", path: wa.path },
      respond: { status: 200, json: { request_id: "req-take-1" } },
    })
    const socket = await openWorkspaceSocket(tenant)
    try {
      const response = await threadControl(api, wa.conversationId, {
        contactInboxId: wa.contactInboxId,
        action: "take",
        metadata: "  Customer asked for a human  ",
      })
      expect(response.status, response.text).toBe(200)
      expect(Object.keys(response.json).sort()).toEqual(["snapshot", "status"])
      expect(Object.keys(response.json.snapshot as object).sort()).toEqual(SNAPSHOT_KEYS)
      expect(n.normalize(response.json)).toEqual({
        status: "applied",
        snapshot: {
          contactInboxId: "<id:1>",
          threadControlState: "owned",
          threadOwnerRole: "escalation",
          threadOwnerAppId: null,
          threadControlUpdatedAt: "<ts>",
          threadOwnerExpiresAt: null,
          threadControlLastEvent: "taken",
        },
      })
      const snapshot = response.json.snapshot as Snapshot
      expect(snapshot.contactInboxId).toBe(wa.contactInboxId)
      // Our own routing times are truncated to whole seconds.
      expect(Date.parse(snapshot.threadControlUpdatedAt as string) % 1000).toBe(0)

      const calls = await recordsOf(GRAPH, wa.path)
      expect(calls).toHaveLength(1)
      expect(calls[0]).toMatchObject({ scheme: "https", method: "POST", unmatched: false })
      expect(calls[0]?.headers.authorization).toBe(`Bearer ${wa.accessToken}`)
      expect(bodyOf(calls[0])).toEqual({
        messaging_product: "whatsapp",
        to: wa.waId,
        action: "take",
        metadata: "Customer asked for a human",
      })

      const { contactInboxId: _id, ...routing } = snapshot
      expect(await storedRouting(api, wa.conversationId)).toEqual(routing)

      const event = await socket.waitFor("contactInboxThreadControlUpdated", (e) =>
        JSON.stringify(e.data).includes(wa.contactInboxId),
      )
      expect(event.data).toEqual({ conversationId: wa.conversationId, ...snapshot })
      const divider = await socket.waitFor(
        "messageCreated",
        (e) => (e.data as { conversationId?: string }).conversationId === wa.conversationId,
      )
      expect(divider.data).toMatchObject({
        conversationId: wa.conversationId,
        contactInboxId: wa.contactInboxId,
        messageType: "activity",
        senderType: "system",
        senderId: null,
        contentType: "text",
        text: "This app took over this conversation",
        createdAt: snapshot.threadControlUpdatedAt,
      })

      const messages = await api.get<{ data: Array<Record<string, unknown>> }>(
        contactPath(idOf(wa.contactId), "/messages"),
      )
      expect(messages.status, messages.text).toBe(200)
      expect(messages.json.data).toHaveLength(1)
      expect(messages.json.data[0]).toMatchObject({
        id: (divider.data as { id: string }).id,
        messageType: "activity",
        senderType: "system",
        text: "This app took over this conversation",
      })
    } finally {
      await socket.close()
    }
  })

  it("pass: control_pass only for a role other than escalation; snapshot standby with the target role", async () => {
    const { tenant, api } = await newTenant("p107-tc-wa-pass")
    // One conversation per case: two own actions in the same whole second tie
    // on time, and the later one can lose by event precedence (see the spec).
    const earlier = new Date(Date.now() - HOUR).toISOString()
    const cases = [
      { body: { targetRole: "marketing", metadata: "for marketing" }, role: "marketing" },
      { body: {}, role: "escalation" },
      { body: { targetRole: "escalation" }, role: "escalation" },
    ]
    for (const entry of cases) {
      const wa = await whatsappConversation(tenant, {
        contactInbox: {
          threadControlState: "owned",
          threadOwnerRole: "escalation",
          threadControlUpdatedAt: earlier,
          lastIncomingMessageAt: earlier,
        },
      })
      await fake.script({
        match: { host: GRAPH, method: "POST", path: wa.path },
        respond: { status: 200, json: {} },
      })
      const response = await threadControl(api, wa.conversationId, {
        contactInboxId: wa.contactInboxId,
        action: "pass",
        ...entry.body,
      })
      expect(response.status, response.text).toBe(200)
      expect(response.json).toMatchObject({
        status: "applied",
        snapshot: {
          contactInboxId: wa.contactInboxId,
          threadControlState: "standby",
          threadOwnerRole: entry.role,
          threadOwnerAppId: null,
          threadOwnerExpiresAt: null,
          threadControlLastEvent: "passed",
        },
      })
      const calls = await recordsOf(GRAPH, wa.path)
      expect(calls.map(bodyOf)).toEqual([
        {
          messaging_product: "whatsapp",
          to: wa.waId,
          action: "pass",
          ...(entry.body.metadata ? { metadata: entry.body.metadata } : {}),
          ...(entry.role === "marketing" ? { control_pass: { target_role: "marketing" } } : {}),
        },
      ])
      expect(await storedRouting(api, wa.conversationId)).toMatchObject({
        threadControlState: "standby",
        threadOwnerRole: entry.role,
        threadControlLastEvent: "passed",
      })
    }
  })

  it("release: no note is sent; snapshot idle with no owner", async () => {
    const { tenant, api } = await newTenant("p107-tc-wa-release")
    const wa = await whatsappConversation(tenant, {
      contactInbox: { threadControlState: "owned", threadOwnerRole: "escalation" },
    })
    await fake.script({
      match: { host: GRAPH, method: "POST", path: wa.path },
      respond: { status: 200, json: { request_id: "req-release" } },
    })
    const response = await threadControl(api, wa.conversationId, {
      contactInboxId: wa.contactInboxId,
      action: "release",
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({
      status: "applied",
      snapshot: {
        contactInboxId: wa.contactInboxId,
        threadControlState: "idle",
        threadOwnerRole: null,
        threadOwnerAppId: null,
        threadOwnerExpiresAt: null,
        threadControlLastEvent: "released",
      },
    })
    const calls = await recordsOf(GRAPH, wa.path)
    expect(calls.map(bodyOf)).toEqual([{ messaging_product: "whatsapp", to: wa.waId, action: "release" }])
  })

  it("a take Meta refuses with 2494191 returns 200 notEscalation and changes nothing", async () => {
    const { tenant, api } = await newTenant("p107-tc-wa-refused")
    const wa = await whatsappConversation(tenant, {
      contactInbox: { threadControlState: "standby", threadOwnerRole: "ai_agent" },
    })
    const before = await storedRouting(api, wa.conversationId)
    await fake.script({
      match: { host: GRAPH, method: "POST", path: wa.path },
      respond: {
        status: 400,
        json: {
          error: {
            message: "Only the escalation partner can take the thread",
            type: "OAuthException",
            code: 2494191,
            fbtrace_id: "parity-trace",
          },
        },
      },
    })
    const response = await threadControl(api, wa.conversationId, { contactInboxId: wa.contactInboxId, action: "take" })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ status: "notEscalation" })
    expect(await recordsOf(GRAPH, wa.path)).toHaveLength(1)
    expect(await storedRouting(api, wa.conversationId)).toEqual(before)
  })

  it("another Meta error -> 400 threadControlFailed; nothing stored", async () => {
    const { tenant, api } = await newTenant("p107-tc-wa-failed")
    const wa = await whatsappConversation(tenant)
    await fake.script({
      match: { host: GRAPH, method: "POST", path: wa.path },
      respond: {
        status: 400,
        json: { error: { message: "Invalid parameter", type: "OAuthException", code: 100, fbtrace_id: "t" } },
      },
    })
    for (const action of ["take", "release"]) {
      expectError(
        await threadControl(api, wa.conversationId, { contactInboxId: wa.contactInboxId, action }),
        400,
        "threadControlFailed",
      )
    }
    // `take` and `release` both called Meta once (no retry).
    expect(await recordsOf(GRAPH, wa.path)).toHaveLength(2)
    expect(await storedRouting(api, wa.conversationId)).toEqual({
      threadControlState: null,
      threadOwnerRole: null,
      threadOwnerAppId: null,
      threadControlUpdatedAt: null,
      threadOwnerExpiresAt: null,
      threadControlLastEvent: null,
    })
  })

  it("sync: WhatsApp cannot report an owner, so the stored state comes back unchanged (no Meta call)", async () => {
    const { tenant, api } = await newTenant("p107-tc-wa-sync")
    const updatedAt = new Date(Date.now() - 60 * 60 * 1000).toISOString()
    const wa = await whatsappConversation(tenant, {
      contactInbox: {
        threadControlState: "standby",
        threadOwnerRole: "marketing",
        threadControlUpdatedAt: updatedAt,
        lastIncomingMessageAt: updatedAt,
      },
    })
    const response = await threadControl(api, wa.conversationId, { contactInboxId: wa.contactInboxId, action: "sync" })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      status: "applied",
      snapshot: {
        contactInboxId: wa.contactInboxId,
        threadControlState: "standby",
        threadOwnerRole: "marketing",
        threadOwnerAppId: null,
        threadControlUpdatedAt: updatedAt,
        threadOwnerExpiresAt: null,
        threadControlLastEvent: null,
      },
    })
    expect(await recordsOf(GRAPH, `/v23.0/${wa.phoneNumberId}`)).toHaveLength(0)
  })

  it("archiving a conversation whose WhatsApp thread we own releases it through the worker", async () => {
    const { tenant, api } = await newTenant("p107-tc-wa-archive")
    const earlier = new Date(Date.now() - HOUR).toISOString()
    const wa = await whatsappConversation(tenant, {
      contactInbox: {
        threadControlState: "owned",
        threadOwnerRole: "escalation",
        threadControlUpdatedAt: earlier,
        lastIncomingMessageAt: earlier,
      },
    })
    await fake.script({
      match: { host: GRAPH, method: "POST", path: wa.path },
      respond: { status: 200, json: { request_id: "req" } },
    })
    expect((await api.post(`/v1/conversations/${wa.conversationId}/archive`)).status).toBe(200)
    const calls = await fake.waitFor({ kind: "http", host: GRAPH, pathPrefix: wa.path }, { count: 1, timeoutMs: 20_000 })
    expect(calls.map(bodyOf)).toEqual([{ messaging_product: "whatsapp", to: wa.waId, action: "release" }])
    expect(calls[0]?.headers.authorization).toBe(`Bearer ${wa.accessToken}`)
    await eventually(
      async () => {
        expect(await storedRouting(api, wa.conversationId)).toMatchObject({
          threadControlState: "idle",
          threadOwnerRole: null,
          threadControlLastEvent: "released",
        })
      },
      { timeoutMs: 20_000, description: "release recorded" },
    )
    expect(await recordsOf(GRAPH, wa.path)).toHaveLength(1)
  })

  it("archiving a conversation whose thread is not owned makes no Meta call", async () => {
    const { tenant, api } = await newTenant("p107-tc-wa-archive-idle")
    const wa = await whatsappConversation(tenant, { contactInbox: { threadControlState: "standby" } })
    // A second conversation in the same workspace is used as a barrier: its
    // owned thread is released by the same worker queue after ours.
    const barrier = await whatsappConversation(tenant)
    await fake.script({
      match: { host: GRAPH, method: "POST", path: barrier.path },
      respond: { status: 200, json: {} },
    })
    expect(
      (await threadControl(api, barrier.conversationId, { contactInboxId: barrier.contactInboxId, action: "take" }))
        .status,
    ).toBe(200)
    expect((await api.post(`/v1/conversations/${wa.conversationId}/archive`)).status).toBe(200)
    expect((await api.post(`/v1/conversations/${barrier.conversationId}/archive`)).status).toBe(200)
    await fake.waitFor({ kind: "http", host: GRAPH, pathPrefix: barrier.path }, { count: 2, timeoutMs: 20_000 })
    expect(await recordsOf(GRAPH, wa.path)).toHaveLength(0)
    expect(await listConversations(api, { tags: ["archived"] }).then((r) => r.json.data.length)).toBe(2)
  })
})

describe("threadControl on Messenger", () => {
  const bodyIncludesPsid = (psid: string) => (record: UpstreamRecord) => record.bodyText.includes(psid)

  it("take calls take_thread_control and makes our app the owner; pass hands it to the Business-AI app", async () => {
    const { tenant, api } = await newTenant("p107-tc-ms")
    const ms = await messengerConversation(tenant)
    await fake.script(
      {
        match: { host: GRAPH, method: "POST", path: "/v23.0/me/take_thread_control", bodyIncludes: ms.psid },
        respond: { status: 200, json: { success: true } },
      },
    )
    const taken = await threadControl(api, ms.conversationId, {
      contactInboxId: ms.contactInboxId,
      action: "take",
      metadata: "agent note",
    })
    expect(taken.status, taken.text).toBe(200)
    expect(taken.json).toMatchObject({
      status: "applied",
      snapshot: {
        contactInboxId: ms.contactInboxId,
        threadControlState: "owned",
        threadOwnerRole: null,
        threadOwnerAppId: P107_META_APP_ID,
        threadOwnerExpiresAt: null,
        threadControlLastEvent: "taken",
      },
    })
    const takes = await recordsOf(GRAPH, "/v23.0/me/take_thread_control", bodyIncludesPsid(ms.psid))
    expect(takes).toHaveLength(1)
    expect(takes[0]?.headers.authorization).toBe(`Bearer ${ms.accessToken}`)
    expect(bodyOf(takes[0])).toEqual({ recipient: { id: ms.psid }, metadata: "agent note" })

    // The pass runs on a second conversation that we have owned for an hour
    // (a pass in the same whole second as the take would tie and lose).
    const earlier = new Date(Date.now() - HOUR).toISOString()
    const owned = await messengerConversation(tenant, {
      contactInbox: {
        threadControlState: "owned",
        threadOwnerAppId: P107_META_APP_ID,
        threadControlUpdatedAt: earlier,
        lastIncomingMessageAt: earlier,
      },
    })
    await fake.script({
      match: { host: GRAPH, method: "POST", path: "/v23.0/me/pass_thread_control", bodyIncludes: owned.psid },
      respond: { status: 200, json: { success: true } },
    })
    const passed = await threadControl(api, owned.conversationId, {
      contactInboxId: owned.contactInboxId,
      action: "pass",
      targetRole: "ai_agent",
    })
    expect(passed.status, passed.text).toBe(200)
    expect(passed.json).toMatchObject({
      status: "applied",
      snapshot: {
        threadControlState: "standby",
        threadOwnerRole: "ai_agent",
        threadOwnerAppId: BUSINESS_AI_APP_ID,
        threadControlLastEvent: "passed",
      },
    })
    const passes = await recordsOf(GRAPH, "/v23.0/me/pass_thread_control", bodyIncludesPsid(owned.psid))
    expect(passes.map(bodyOf)).toEqual([{ recipient: { id: owned.psid }, target_app_id: BUSINESS_AI_APP_ID }])
    expect(await recordsOf(GRAPH, "/v23.0/me/pass_thread_control", bodyIncludesPsid(ms.psid))).toHaveLength(0)
  })

  it("release and a pass to a partner role are refused before any Meta call -> 400 threadControlFailed", async () => {
    const { tenant, api } = await newTenant("p107-tc-ms-refuse")
    const ms = await messengerConversation(tenant)
    expectError(
      await threadControl(api, ms.conversationId, { contactInboxId: ms.contactInboxId, action: "release" }),
      400,
      "threadControlFailed",
    )
    expectError(
      await threadControl(api, ms.conversationId, {
        contactInboxId: ms.contactInboxId,
        action: "pass",
        targetRole: "marketing",
      }),
      400,
      "threadControlFailed",
    )
    const calls = (await fake.requests({ kind: "http", host: GRAPH })).filter((r) => r.bodyText.includes(ms.psid))
    expect(calls).toHaveLength(0)
    expect((await storedRouting(api, ms.conversationId)).threadControlState).toBeNull()
  })

  it("sync asks thread_owner: our app -> owned; the Business-AI app -> standby ai_agent with Meta's expiry", async () => {
    const { tenant, api } = await newTenant("p107-tc-ms-sync")
    const ms = await messengerConversation(tenant)
    const expiration = Math.floor(Date.now() / 1000) + 3600
    await fake.script({
      match: { host: GRAPH, method: "GET", path: "/v23.0/me/thread_owner" },
      respond: { status: 200, json: { data: [{ thread_owner: { app_id: P107_META_APP_ID } }] } },
      times: 1,
    })
    const owned = await threadControl(api, ms.conversationId, { contactInboxId: ms.contactInboxId, action: "sync" })
    expect(owned.status, owned.text).toBe(200)
    expect(owned.json).toMatchObject({
      status: "applied",
      snapshot: {
        contactInboxId: ms.contactInboxId,
        threadControlState: "owned",
        threadOwnerRole: null,
        threadOwnerAppId: null,
        threadOwnerExpiresAt: null,
        threadControlLastEvent: "inboundReceived",
      },
    })
    await fake.script({
      match: { host: GRAPH, method: "GET", path: "/v23.0/me/thread_owner" },
      respond: {
        status: 200,
        json: { data: [{ thread_owner: { app_id: Number(BUSINESS_AI_APP_ID), expiration } }] },
      },
      times: 1,
    })
    const standby = await threadControl(api, ms.conversationId, { contactInboxId: ms.contactInboxId, action: "sync" })
    expect(standby.status, standby.text).toBe(200)
    expect(standby.json).toMatchObject({
      status: "applied",
      snapshot: {
        threadControlState: "standby",
        threadOwnerRole: "ai_agent",
        threadOwnerAppId: BUSINESS_AI_APP_ID,
        threadOwnerExpiresAt: new Date(expiration * 1000).toISOString(),
        threadControlLastEvent: "standbyReceived",
      },
    })
    const asks = (await recordsOf(GRAPH, "/v23.0/me/thread_owner")).filter((r) => r.query.recipient === ms.psid)
    expect(asks).toHaveLength(2)
    expect(asks[0]).toMatchObject({ method: "GET", unmatched: false })
    expect(asks[0]?.headers.authorization).toBe(`Bearer ${ms.accessToken}`)
    const { contactInboxId: _id, ...routing } = standby.json.snapshot as Snapshot
    expect(await storedRouting(api, ms.conversationId)).toEqual(routing)
  })
})

describe("threadControl on a channel without thread control (webchat)", () => {
  it("take -> 400 threadControlUnsupported; sync -> applied with the unchanged (empty) snapshot", async () => {
    const { api } = await newTenant("p107-tc-webchat")
    const webchat = await createWebchat(api)
    const name = `Tc${Date.now().toString(36)}`
    await createWebchatContact(api, webchat.inboxId, { firstName: name })
    const [item] = (await listConversations(api, { keyword: name })).json.data as [ConversationItem]
    const contactInboxId = item.contactInboxes[0]?.id as string
    for (const action of ["take", "release", "pass"]) {
      expectError(await threadControl(api, item.id, { contactInboxId, action }), 400, "threadControlUnsupported", {
        message: "This channel does not support thread control.",
      })
    }
    const synced = await threadControl(api, item.id, { contactInboxId, action: "sync" })
    expect(synced.status, synced.text).toBe(200)
    expect(synced.json).toEqual({
      status: "applied",
      snapshot: {
        contactInboxId,
        threadControlState: null,
        threadOwnerRole: null,
        threadOwnerAppId: null,
        threadControlUpdatedAt: null,
        threadOwnerExpiresAt: null,
        threadControlLastEvent: null,
      },
    })
  })
})

describe("threadControl errors", () => {
  it("404 for a contact inbox of another contact, an unknown contact inbox, or an unknown conversation", async () => {
    const { tenant, api } = await newTenant("p107-tc-404")
    const wa = await whatsappConversation(tenant)
    const other = await seedConversation(tenant.workspaceId)
    for (const action of ["take", "sync"]) {
      for (const contactInboxId of [other.contactInboxId, UNKNOWN_ID]) {
        expectError(await threadControl(api, wa.conversationId, { contactInboxId, action }), 404, "notFound", {
          message: "Contact inbox not found for this conversation",
        })
      }
      expectError(
        await threadControl(api, UNKNOWN_ID, { contactInboxId: wa.contactInboxId, action }),
        404,
        "notFound",
        { message: "Conversation not found" },
      )
    }
    expect(await recordsOf(GRAPH, wa.path)).toHaveLength(0)
  })

  it("422 for invalid input (action, contactInboxId, targetRole / metadata rules)", async () => {
    const { tenant, api } = await newTenant("p107-tc-422")
    const wa = await whatsappConversation(tenant)
    const base = { contactInboxId: wa.contactInboxId }
    const post = (body: Record<string, unknown>) => threadControl(api, wa.conversationId, body)
    expectInvalid(await post({ ...base, action: "steal" }), "action")
    expectInvalid(await post({ action: "take" }), "contactInboxId")
    expectInvalid(await post({ contactInboxId: "abc", action: "take" }), "contactInboxId")
    expectInvalid(await post({ ...base, action: "take", targetRole: "marketing" }), "targetRole")
    expectInvalid(await post({ ...base, action: "release", targetRole: "marketing" }), "targetRole")
    expectInvalid(await post({ ...base, action: "pass", targetRole: "boss" }), "targetRole")
    expectInvalid(await post({ ...base, action: "release", metadata: "note" }), "metadata")
    expectInvalid(await post({ ...base, action: "sync", metadata: "note" }), "metadata")
    expectInvalid(await post({ ...base, action: "take", metadata: "   " }), "metadata")
    expectInvalid(await post({ ...base, action: "take", metadata: "x".repeat(2001) }), "metadata")
    expectInvalid(
      await api.post(`/v1/conversations/abc/thread-control`, { ...base, action: "take" }),
      "id",
    )
    expect(await recordsOf(GRAPH, wa.path)).toHaveLength(0)
  })

  it("a 2000-character note is accepted and sent in full", async () => {
    const { tenant, api } = await newTenant("p107-tc-note")
    const wa = await whatsappConversation(tenant)
    await fake.script({ match: { host: GRAPH, method: "POST", path: wa.path }, respond: { status: 200, json: {} } })
    const note = "n".repeat(2000)
    const response = await threadControl(api, wa.conversationId, {
      contactInboxId: wa.contactInboxId,
      action: "take",
      metadata: note,
    })
    expect(response.status, response.text).toBe(200)
    expect((bodyOf((await recordsOf(GRAPH, wa.path))[0]) as { metadata: string }).metadata).toBe(note)
  })
})
