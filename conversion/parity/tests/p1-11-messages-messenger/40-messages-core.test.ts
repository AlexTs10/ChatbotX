// messages.create / list / get / delete / changeAttributes / update /
// getFlowResponse on webchat and API-channel conversations (scope `inbox`).
// Webchat sends need no third party; the API channel delivers the message to
// its callback URL, a test-owned host on the fake upstream. Side effects
// checked: the stored message (list/get), the realtime `messageCreated` /
// `messageDeleted` events, the callback request, the worker's flow output and
// the attachment copy in storage.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { Normalizer } from "../../src/normalize.ts"
import {
  createApiChannel,
  createPublishedFlow,
  ext,
  expectError,
  expectInvalid,
  expectInvalidAny,
  expectNotFound,
  expectSystemError,
  fetchPublic,
  getMessage,
  keysOf,
  listMessages,
  MESSAGE_KEYS,
  type Message,
  memberSocket,
  messagesPath,
  newTenant,
  PNG_1X1,
  parseBody,
  seedConversation,
  UNKNOWN_ID,
  uniqueName,
  upstreamCalls,
  uploadMediaFile,
  wait,
  webchatConversation,
} from "./helpers.ts"

describe("messages.create on a webchat conversation", () => {
  it("creates an outgoing api text message (201) with the full shape; list and get return it", async () => {
    const { tenant, api } = await newTenant("p111-msg-create")
    const conversation = await webchatConversation(tenant, api)
    const n = new Normalizer()
    expect(await listMessages(api, conversation.conversationId)).toEqual({ data: [], nextCursor: null, prevCursor: null })

    const response = await api.post<Message>(messagesPath(conversation.conversationId), {
      text: "  Hello from P1.11  ",
      clientId: "123456789",
    })
    expect(response.status, response.text).toBe(201)
    expect(keysOf(response.json)).toEqual(MESSAGE_KEYS)
    expect(
      n.normalize(response.json, { replace: { [tenant.workspaceId]: "<workspace>" } }),
    ).toEqual({
      contactInboxId: "<id:1>",
      conversationId: "<id:2>",
      id: "<id:3>",
      workspaceId: "<workspace>",
      text: "Hello from P1.11",
      messageType: "outgoing",
      contentType: "text",
      senderType: "api",
      senderId: null,
      sourceId: null,
      type: "message",
      parentId: null,
      attributes: null,
      contentAttributes: null,
      deletedAt: null,
      sendError: null,
      attachments: [],
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })
    expect(response.json.conversationId).toBe(conversation.conversationId)
    expect(response.json.contactInboxId).toBe(conversation.contactInboxId)

    const page = await listMessages(api, conversation.conversationId)
    expect(page.nextCursor).toBeNull()
    expect(page.data).toHaveLength(1)
    expect(page.data[0]).toMatchObject({ id: response.json.id, text: "Hello from P1.11", senderType: "api" })
    expect(keysOf(page.data[0])).toEqual(MESSAGE_KEYS)

    const one = await getMessage(api, conversation.conversationId, response.json)
    expect(one.status, one.text).toBe(200)
    expect(one.json).toMatchObject({ id: response.json.id, text: "Hello from P1.11", createdAt: response.json.createdAt })
    expect(keysOf(one.json)).toEqual(MESSAGE_KEYS)
  })

  it("the workspace socket gets messageCreated with the clientId echoed", async () => {
    const { tenant, api } = await newTenant("p111-msg-rt")
    const conversation = await webchatConversation(tenant, api)
    const socket = await memberSocket(tenant)
    try {
      const text = uniqueName("Realtime")
      const sent = await api.post<Message>(messagesPath(conversation.conversationId), { text, clientId: "987654321" })
      expect(sent.status, sent.text).toBe(201)
      const event = await socket.waitFor("messageCreated", (e) => JSON.stringify(e.data).includes(text), 15_000)
      expect(event.data).toMatchObject({
        id: sent.json.id,
        text,
        messageType: "outgoing",
        senderType: "api",
        conversationId: conversation.conversationId,
        clientId: "987654321",
      })
    } finally {
      await socket.close()
    }
  })

  it("errors: unknown or foreign conversation 404, inbox not on the contact 404, invalid bodies 422", async () => {
    const { tenant, api } = await newTenant("p111-msg-create-err")
    const conversation = await webchatConversation(tenant, api)
    const path = messagesPath(conversation.conversationId)
    // `create` does not declare `notFound`, so its 404s are `defined: false`.
    expectNotFound(await api.post(messagesPath(UNKNOWN_ID), { text: "x" }), "Conversation not found", false)
    const { api: otherApi } = await newTenant("p111-msg-create-err-o")
    expectNotFound(await otherApi.post(path, { text: "x" }), "Conversation not found", false)
    expectNotFound(await api.post(path, { text: "x", inboxId: UNKNOWN_ID }), "Inbox not found", false)
    expectInvalidAny(await api.post(path, {}))
    expectInvalidAny(await api.post(path, { text: "   " }))
    expectInvalidAny(await api.post(path, { text: "x".repeat(1001) }))
    expectInvalidAny(await api.post(path, { text: "x", inboxId: "abc" }))
    expectInvalid(await api.post(messagesPath("abc"), { text: "x" }), "conversationId")
    expect((await listMessages(api, conversation.conversationId)).data).toEqual([])
  })

  it("a flowId sends the flow (201, body null); the worker writes the flow's message", async () => {
    const { tenant, api } = await newTenant("p111-msg-flow")
    const conversation = await webchatConversation(tenant, api)
    const flowText = uniqueName("Flow says hi")
    const flowId = await createPublishedFlow(api, uniqueName("Send flow"), flowText)
    const response = await api.post(messagesPath(conversation.conversationId), { flowId })
    expect(response.status, response.text).toBe(201)
    expect(response.text).toBe("null")
    await wait(async () => {
      const texts = (await listMessages(api, conversation.conversationId)).data.map((message) => message.text)
      expect(texts).toContain(flowText)
    })
    expectNotFound(await api.post(messagesPath(conversation.conversationId), { flowId: UNKNOWN_ID }), "Flow not found", false)
  })

  it("products on a channel without product messages -> 422 productMessageUnresolvable channelUnsupported", async () => {
    const { tenant, api } = await newTenant("p111-msg-products")
    const conversation = await webchatConversation(tenant, api)
    const response = await api.post(messagesPath(conversation.conversationId), {
      products: { display: "products", body: "Our picks", sections: [{ productIds: [UNKNOWN_ID] }] },
    })
    const body = expectError(response, 422, "productMessageUnresolvable", {
      defined: true,
      message: "Product message cannot be sent: channelUnsupported (reason=channelUnsupported)",
    })
    expect(body.data).toEqual({ reason: "channelUnsupported" })
    expect((await listMessages(api, conversation.conversationId)).data).toEqual([])
  })

  it("mediaFileId copies the library file into the conversation; the attachment url serves the bytes", async () => {
    const { tenant, api } = await newTenant("p111-msg-media")
    const conversation = await webchatConversation(tenant, api)
    const file = await uploadMediaFile(api, { name: "cat.png" })
    const response = await api.post<Message>(messagesPath(conversation.conversationId), {
      text: "With a picture",
      mediaFileId: file.id,
    })
    expect(response.status, response.text).toBe(201)
    expect(response.json.attachments).toHaveLength(1)
    const [attachment] = response.json.attachments as [Record<string, unknown>]
    expect(attachment).toMatchObject({ fileType: "image", mimeType: "image/png", name: "cat.png", size: PNG_1X1.length })
    const prefix = `public/space/${tenant.workspaceId}/conversations/${conversation.conversationId}/`
    expect(String(attachment.originPath)).toMatch(new RegExp(`^${prefix}\\d{15,20}`))
    expect(attachment.url).toBe(`http://app/storage/${attachment.originPath}`)
    const stored = await fetchPublic(String(attachment.url))
    expect(stored.status).toBe(200)
    expect(stored.bytes.equals(PNG_1X1)).toBe(true)

    // Deleting the library file keeps the sent copy.
    expect((await api.delete(`/v1/media-library/files/${file.id}`)).status).toBe(204)
    expect((await fetchPublic(String(attachment.url))).status).toBe(200)

    // A file id of another workspace (or unknown) is refused.
    const { api: otherApi } = await newTenant("p111-msg-media-o")
    const foreign = await uploadMediaFile(otherApi)
    expectSystemError(
      await api.post(messagesPath(conversation.conversationId), { text: "x", mediaFileId: foreign.id }),
      "Media library file not found",
    )
    expectSystemError(
      await api.post(messagesPath(conversation.conversationId), { mediaFileIds: [UNKNOWN_ID] }),
      "Media library file not found",
    )
  })
})

describe("messages.list paging", () => {
  it("newest first, 20 per page by default; the cursor walks back; perPage over 50 is 422", async () => {
    const { tenant, api } = await newTenant("p111-msg-paging")
    const conversation = await webchatConversation(tenant, api)
    const fx = await ext()
    const start = Date.now()
    for (let index = 0; index < 23; index += 1) {
      await fx.createMessage({
        workspaceId: tenant.workspaceId,
        conversationId: conversation.conversationId,
        contactInboxId: conversation.contactInboxId,
        messageType: index % 2 === 0 ? "incoming" : "outgoing",
        senderType: index % 2 === 0 ? "contact" : "api",
        text: `m${index}`,
        createdAt: new Date(start + index * 10).toISOString(),
      })
    }
    const first = await listMessages(api, conversation.conversationId)
    expect(first.data.map((message) => message.text)).toEqual(
      Array.from({ length: 20 }, (_, i) => `m${22 - i}`),
    )
    expect(typeof first.nextCursor).toBe("string")
    expect(first.prevCursor).toBeNull()
    const second = await listMessages(api, conversation.conversationId, { cursor: first.nextCursor as string })
    expect(second.data.map((message) => message.text)).toEqual(["m2", "m1", "m0"])
    expect(second.nextCursor).toBeNull()

    const small = await listMessages(api, conversation.conversationId, { perPage: 5 })
    expect(small.data.map((message) => message.text)).toEqual(["m22", "m21", "m20", "m19", "m18"])
    const big = await listMessages(api, conversation.conversationId, { perPage: 50 })
    expect(big.data).toHaveLength(23)
    expect(big.nextCursor).toBeNull()
    expect(big.data[0]).toMatchObject({ messageType: "incoming", senderType: "contact", text: "m22" })
    expect(big.data[1]).toMatchObject({ messageType: "outgoing", senderType: "api", text: "m21" })

    expectInvalid(await api.get(messagesPath(conversation.conversationId), { query: { perPage: 51 } }), "perPage")
    expectInvalid(await api.get(messagesPath(conversation.conversationId), { query: { perPage: 0 } }), "perPage")
    expectNotFound(await api.get(messagesPath(UNKNOWN_ID)), "Conversation not found")
    const { api: otherApi } = await newTenant("p111-msg-paging-o")
    expectNotFound(await otherApi.get(messagesPath(conversation.conversationId)), "Conversation not found")
  })
})

describe("messages.get", () => {
  it("needs the message's createdAt; another conversation's message or a wrong createdAt is 404", async () => {
    const { tenant, api } = await newTenant("p111-msg-get")
    const conversation = await webchatConversation(tenant, api)
    const other = await seedConversation(tenant, { inboxId: conversation.inboxId, channel: "webchat" })
    const sent = await api.post<Message>(messagesPath(conversation.conversationId), { text: "Find me" })
    expect(sent.status, sent.text).toBe(201)
    expect((await getMessage(api, conversation.conversationId, sent.json)).status).toBe(200)
    expectNotFound(await getMessage(api, other.conversationId, sent.json), "Message not found")
    expectNotFound(
      await getMessage(api, conversation.conversationId, {
        id: sent.json.id,
        createdAt: new Date(Date.parse(sent.json.createdAt) - 60_000).toISOString(),
      }),
      "Message not found",
    )
    expectNotFound(await getMessage(api, conversation.conversationId, { id: UNKNOWN_ID, createdAt: sent.json.createdAt }), "Message not found")
    expectInvalid(await api.get(messagesPath(conversation.conversationId, `/${sent.json.id}`)), "createdAt")
    expectInvalid(
      await api.get(messagesPath(conversation.conversationId, `/${sent.json.id}`), { query: { createdAt: "not-a-date" } }),
      "createdAt",
    )
    expectInvalid(await api.get(messagesPath(conversation.conversationId, "/abc"), { query: { createdAt: sent.json.createdAt } }), "messageId")
  })
})

describe("messages.delete / changeAttributes / update on a chat message", () => {
  it("delete soft-deletes (204) and emits messageDeleted; the message stays readable with deletedAt", async () => {
    const { tenant, api } = await newTenant("p111-msg-delete")
    const conversation = await webchatConversation(tenant, api)
    const sent = await api.post<Message>(messagesPath(conversation.conversationId), { text: "Delete me" })
    expect(sent.status, sent.text).toBe(201)
    const socket = await memberSocket(tenant)
    try {
      const response = await api.delete(messagesPath(conversation.conversationId, `/${sent.json.id}`), {
        json: { createdAt: sent.json.createdAt },
      })
      expect(response.status, response.text).toBe(204)
      expect(response.text).toBe("")
      const event = await socket.waitFor("messageDeleted", (e) => JSON.stringify(e.data).includes(sent.json.id), 15_000)
      expect(event.data).toEqual({ messageIds: [sent.json.id] })
    } finally {
      await socket.close()
    }
    const one = await getMessage(api, conversation.conversationId, sent.json)
    expect(one.status, one.text).toBe(200)
    expect(one.json.deletedAt).toMatch(/^\d{4}-\d{2}-\d{2}T/)
    const listed = (await listMessages(api, conversation.conversationId)).data
    expect(listed.map((message) => message.id)).toEqual([sent.json.id])
    expect(listed[0]?.deletedAt).toBe(one.json.deletedAt)
  })

  it("delete errors: unknown message 400 systemError, missing createdAt 422, foreign conversation 404", async () => {
    const { tenant, api } = await newTenant("p111-msg-delete-err")
    const conversation = await webchatConversation(tenant, api)
    const sent = await api.post<Message>(messagesPath(conversation.conversationId), { text: "Stay" })
    const path = messagesPath(conversation.conversationId, `/${sent.json.id}`)
    expectSystemError(
      await api.delete(messagesPath(conversation.conversationId, `/${UNKNOWN_ID}`), { json: { createdAt: sent.json.createdAt } }),
      "Comment not found",
    )
    expectInvalid(await api.delete(path, { json: {} }), "createdAt")
    const { api: otherApi } = await newTenant("p111-msg-delete-err-o")
    expectNotFound(await otherApi.delete(path, { json: { createdAt: sent.json.createdAt } }), "Conversation not found")
    expect((await getMessage(api, conversation.conversationId, sent.json)).json.deletedAt).toBeNull()
  })

  it("changeAttributes answers 200 for a chat message but the worker leaves non-comments alone", async () => {
    const { tenant, api } = await newTenant("p111-msg-attrs")
    const conversation = await webchatConversation(tenant, api)
    const sent = await api.post<Message>(messagesPath(conversation.conversationId), { text: "Like me" })
    const path = messagesPath(conversation.conversationId, `/${sent.json.id}/attributes`)
    const response = await api.post(path, { createdAt: sent.json.createdAt, liked: true, hidden: true })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ success: true, messageId: sent.json.id })
    expect((await getMessage(api, conversation.conversationId, sent.json)).json.attributes).toBeNull()
    expectSystemError(
      await api.post(messagesPath(conversation.conversationId, `/${UNKNOWN_ID}/attributes`), {
        createdAt: sent.json.createdAt,
        liked: true,
      }),
      "Message not found",
    )
    expectInvalid(await api.post(path, { liked: true }), "createdAt")
    expectInvalid(await api.post(path, { createdAt: sent.json.createdAt, liked: "yes" }), "liked")
  })

  it("update (edit) refuses a chat message: 400 'Message is not an editable comment'", async () => {
    const { tenant, api } = await newTenant("p111-msg-edit-chat")
    const conversation = await webchatConversation(tenant, api)
    const sent = await api.post<Message>(messagesPath(conversation.conversationId), { text: "Original" })
    const path = messagesPath(conversation.conversationId, `/${sent.json.id}`)
    expectSystemError(await api.put(path, { createdAt: sent.json.createdAt, newText: "Edited" }), "Message is not an editable comment")
    expectSystemError(
      await api.put(messagesPath(conversation.conversationId, `/${UNKNOWN_ID}`), { createdAt: sent.json.createdAt, newText: "x" }),
      "Comment not found",
    )
    expectInvalid(await api.put(path, { createdAt: sent.json.createdAt, newText: "" }), "newText")
    expectInvalid(await api.put(path, { createdAt: sent.json.createdAt, newText: "x".repeat(2001) }), "newText")
    expectInvalid(await api.put(path, { newText: "x" }), "createdAt")
    expect((await getMessage(api, conversation.conversationId, sent.json)).json.text).toBe("Original")
  })
})

describe("messages.getFlowResponse", () => {
  it("is 404 for a message that is not a WhatsApp Flow response, and for a foreign conversation path", async () => {
    const { tenant, api } = await newTenant("p111-msg-flowresp")
    const conversation = await webchatConversation(tenant, api)
    const other = await seedConversation(tenant, { inboxId: conversation.inboxId, channel: "webchat" })
    const sent = await api.post<Message>(messagesPath(conversation.conversationId), { text: "Plain" })
    const path = messagesPath(conversation.conversationId, `/${sent.json.id}/flow-response`)
    expectNotFound(await api.get(path, { query: { createdAt: sent.json.createdAt } }), "WhatsApp Flow response not found")
    expectNotFound(
      await api.get(messagesPath(other.conversationId, `/${sent.json.id}/flow-response`), {
        query: { createdAt: sent.json.createdAt },
      }),
      "Message not found",
    )
    expectInvalid(await api.get(path), "createdAt")
  })
})

describe("messages.create on an API-channel conversation", () => {
  it("delivers the message to the channel's callback URL", async () => {
    const { tenant, api } = await newTenant("p111-msg-api")
    const channel = await createApiChannel(api)
    const conversation = await seedConversation(tenant, { inboxId: channel.inboxId, channel: "api" })
    const since = await fake.cursor()
    const text = uniqueName("To the API")
    const sent = await api.post<Message>(messagesPath(conversation.conversationId), { text })
    expect(sent.status, sent.text).toBe(201)
    expect(sent.json).toMatchObject({ text, messageType: "outgoing", senderType: "api" })
    const [call] = await wait(async () => {
      const calls = (await upstreamCalls({ host: channel.callbackHost, since, method: "POST" })).filter(
        (record) => record.path === "/hook",
      )
      expect(calls).toHaveLength(1)
      return calls
    })
    const body = parseBody<{
      event: string
      contact: { id: string; sourceId: string }
      message: { text: string; messageType: string }
    }>(call as NonNullable<typeof call>)
    expect(body).toMatchObject({
      event: "message_created",
      contact: { sourceId: conversation.sourceId },
      message: { text, messageType: "outgoing", contentType: "text" },
    })
    expect(call?.headers["x-chatbotx-signature"]).toMatch(/^sha256=[0-9a-f]{64}$/)
  })
})
