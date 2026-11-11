// contacts.sendMessage / listMessages / getMessage (scope `inbox`) and
// contacts.sendFlow / triggerAutoReply (scope `automation`), on webchat
// contacts (no third-party call). Side effects: stored messages, the
// realtime `messageCreated` event, flow output written by the worker.

import { beforeAll, describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { signedInClient, type Tenant } from "../../src/fixtures/index.ts"
import { expectNormalized, Normalizer } from "../../src/normalize.ts"
import { connectRealtime, workspaceSocketUrl } from "../../src/realtime.ts"
import { rpc } from "../../src/rpc.ts"
import {
  type Api,
  contactPath,
  createPublishedFlow,
  createWebchat,
  createWebchatContact,
  expectError,
  expectInvalid,
  ext,
  idOf,
  newTenant,
  qs,
} from "./helpers.ts"

type Message = {
  id: string
  conversationId: string
  contactInboxId: string
  text: string | null
  messageType: string
  senderType: string
  [key: string]: unknown
}
type MessagePage = { data: Message[]; nextCursor: string | null; prevCursor: string | null }

let tenant: Tenant
let api: Api
let inboxId: string

beforeAll(async () => {
  ;({ tenant, api } = await newTenant("p106-messages"))
  inboxId = (await createWebchat(api)).inboxId
})

const messagesOf = async (contactId: string, query: Record<string, string | number | undefined> = {}) => {
  const response = await api.get<MessagePage>(`${contactPath(idOf(contactId), "/messages")}${qs(query)}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("sendMessage / listMessages / getMessage", () => {
  it("sends a text (204); it is listed as an outgoing api message and readable by id", async () => {
    const n = new Normalizer()
    const contactId = await createWebchatContact(api, inboxId, { firstName: "Chat" })
    expect(await messagesOf(contactId)).toEqual({ data: [], nextCursor: null, prevCursor: null })
    const sent = await api.post(contactPath(idOf(contactId), "/messages"), { text: "Hello from the API" })
    expect(sent.status, sent.text).toBe(204)
    const page = await messagesOf(contactId)
    expect(page.data).toHaveLength(1)
    const [message] = page.data as [Message]
    expect(message).toMatchObject({
      text: "Hello from the API",
      messageType: "outgoing",
      contentType: "text",
      senderType: "api",
      senderId: null,
      workspaceId: tenant.workspaceId,
      attachments: [],
    })
    expectNormalized(n, page, { replace: { [tenant.workspaceId]: "<workspace>" } }).toMatchSnapshot()
    const one = await api.get<Message>(contactPath(idOf(contactId), `/messages/${message.id}`))
    expect(one.status, one.text).toBe(200)
    expect(one.json).toEqual(message)
  })

  it("pages newest first with a cursor; contactInboxId narrows to one channel", async () => {
    const contactId = await createWebchatContact(api, inboxId, { firstName: "Pager" })
    for (const text of ["m1", "m2", "m3"]) {
      expect((await api.post(contactPath(idOf(contactId), "/messages"), { text })).status).toBe(204)
    }
    const first = await messagesOf(contactId, { perPage: 2 })
    expect(first.data.map((m) => m.text)).toEqual(["m3", "m2"])
    expect(typeof first.nextCursor).toBe("string")
    expect(first.prevCursor).toBeNull()
    const second = await messagesOf(contactId, { perPage: 2, cursor: first.nextCursor as string })
    expect(second.data.map((m) => m.text)).toEqual(["m1"])
    expect(second.nextCursor).toBeNull()

    const own = first.data[0]?.contactInboxId as string
    expect((await messagesOf(contactId, { contactInboxId: own })).data).toHaveLength(3)
    const other = await createWebchatContact(api, inboxId, { firstName: "Other" })
    const otherInbox = await api.get<{ data: Array<{ id: string }> }>(contactPath(idOf(other), "/inboxes"))
    expect((await messagesOf(contactId, { contactInboxId: otherInbox.json.data[0]?.id })).data).toEqual([])
  })

  it("the workspace realtime socket gets messageCreated for the sent message", async () => {
    const contactId = await createWebchatContact(api, inboxId, { firstName: "Live" })
    const http = await signedInClient(tenant)
    const minted = await rpc<{ token: string }>(
      http,
      "realtimeAPI/mintWorkspaceConnectTokenAuthenticatedAPI",
      { workspaceId: tenant.workspaceId },
      { method: "GET" },
    )
    expect(minted.status).toBe(200)
    const socket = connectRealtime(workspaceSocketUrl(tenant.workspaceId, minted.json.token))
    try {
      await socket.opened
      expect((await api.post(contactPath(idOf(contactId), "/messages"), { text: "Realtime hello" })).status).toBe(204)
      const event = await socket.waitFor(
        "messageCreated",
        (e) => JSON.stringify(e.data).includes("Realtime hello"),
        15_000,
      )
      expect(event.data).toMatchObject({ text: "Realtime hello", messageType: "outgoing" })
    } finally {
      await socket.close()
    }
  })

  it("errors: no conversation, unknown message, message of another contact, bad input", async () => {
    const contactId = await createWebchatContact(api, inboxId, { firstName: "Err" })
    const other = await createWebchatContact(api, inboxId, { firstName: "Err2" })
    expect((await api.post(contactPath(idOf(other), "/messages"), { text: "theirs" })).status).toBe(204)
    const theirs = (await messagesOf(other)).data[0] as Message
    expectError(await api.get(contactPath(idOf(contactId), `/messages/${theirs.id}`)), 404, "notFound")
    expectError(await api.get(contactPath(idOf(contactId), "/messages/1")), 404, "notFound")
    expectInvalid(await api.get(contactPath(idOf(contactId), "/messages/abc")), "messageId")

    const fx = await ext()
    const noConversation = await fx.createContact({ workspaceId: tenant.workspaceId, firstName: "Silent", conversation: null })
    expectError(await api.post(contactPath(idOf(noConversation.contactId), "/messages"), { text: "x" }), 404, "notFound", {
      message: "Conversation not found",
    })
    expectError(await api.get(contactPath(idOf(noConversation.contactId), "/messages")), 404, "notFound", {
      message: "Conversation not found",
    })
    expectError(await api.post(contactPath("id:1", "/messages"), { text: "x" }), 404, "notFound")
    // An inbox the contact is not on.
    expectError(
      await api.post(contactPath(idOf(contactId), "/messages"), { text: "x", inboxId: "1" }),
      404,
      "notFound",
      { message: "Conversation not found" },
    )
    const empty = await api.post(contactPath(idOf(contactId), "/messages"), {})
    expect(empty.status, empty.text).toBe(422)
    expect((await messagesOf(contactId)).data).toEqual([])
  })
})

describe("sendFlow / triggerAutoReply", () => {
  it("sendFlow queues the flow (204); the worker writes its message", async () => {
    const contactId = await createWebchatContact(api, inboxId, { firstName: "Flowy" })
    const flowId = await createPublishedFlow(api, "Parity send flow", "Flow says hi")
    const response = await api.post(contactPath(idOf(contactId), "/flows"), { flowId })
    expect(response.status, response.text).toBe(204)
    await eventually(
      async () => {
        const texts = (await messagesOf(contactId)).data.map((m) => m.text)
        expect(texts).toContain("Flow says hi")
      },
      { timeoutMs: 20_000, intervalMs: 300, description: "flow message" },
    )
  })

  it("sendFlow errors", async () => {
    const contactId = await createWebchatContact(api, inboxId, { firstName: "Flowless" })
    const unknown = await api.post(contactPath(idOf(contactId), "/flows"), { flowId: "1" })
    expect(unknown.status, unknown.text).toBe(404)
    expectInvalid(await api.post(contactPath(idOf(contactId), "/flows"), {}), "flowId")
    expectInvalid(await api.post(contactPath(idOf(contactId), "/flows"), { flowId: "abc" }), "flowId")
    expectError(await api.post(contactPath("id:1", "/flows"), { flowId: "1" }), 404, "notFound")
  })

  it("triggerAutoReply sends the matching keyword's text; no match is 404", async () => {
    const contactId = await createWebchatContact(api, inboxId, { firstName: "Keyword" })
    const keyword = await api.post("/v1/keywords", { type: "inbound", keywords: ["parity hello"], text: "Auto hi" })
    expect(keyword.status, keyword.text).toBe(201)
    const response = await api.post(contactPath(idOf(contactId), "/auto-replies"), { keyword: "parity hello" })
    expect(response.status, response.text).toBe(204)
    await eventually(async () => {
      const page = await messagesOf(contactId)
      expect(page.data.map((m) => m.text)).toContain("Auto hi")
    })
    // Exact match only.
    expectError(await api.post(contactPath(idOf(contactId), "/auto-replies"), { keyword: "parity" }), 404, "notFound", {
      message: "No automated response found for this keyword",
    })
    expectInvalid(await api.post(contactPath(idOf(contactId), "/auto-replies"), { keyword: "" }), "keyword")
    expectError(await api.post(contactPath("id:1", "/auto-replies"), { keyword: "parity hello" }), 404, "notFound")
  })
})
