// messages.* on Messenger conversations, with the outbound Graph API calls
// checked through the fake upstream:
// - create (chat): `POST /v23.0/me/messages` with the page token, recipient =
//   the contact's PSID; Meta's `message_id` becomes the row's `sourceId`.
// - create (comment reply, `replyToMessageId`): `POST /v23.0/<parent comment
//   id>/comments`; the reply's comment id becomes its `sourceId`
//   (realtime `messageIdAssigned`).
// - update (edit a comment reply): `POST /v23.0/<comment id>` {message}
//   (realtime `messageUpdated`).
// - changeAttributes: liked -> `POST|DELETE /<comment id>/likes`, hidden ->
//   `POST /<comment id>` {is_hidden}; the stored `attributes` change.
// - delete: `DELETE /v23.0/<comment id>` (realtime `messageDeleted`).
// Page and comment ids are unique per test, so every script is test-owned.
// The parent (incoming) comment comes from the fixture extension: an inbound
// comment has no public HTTP path except the Meta webhook (P1.2x units).

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import {
  type Api,
  connectMessenger,
  type Conversation,
  expectError,
  expectSystemError,
  ext,
  GRAPH,
  getMessage,
  graphError,
  graphPath,
  keysOf,
  type Message,
  type MessengerPage,
  memberSocket,
  messagesPath,
  newTenant,
  parseBody,
  reEscape,
  seedConversation,
  uniqueDigits,
  uniqueName,
  upstreamCalls,
  wait,
  waitForCalls,
} from "./helpers.ts"

type Setup = { tenant: Tenant; api: Api; page: MessengerPage; conversation: Conversation }

const setup = async (label: string): Promise<Setup> => {
  const { tenant, api } = await newTenant(label)
  const page = await connectMessenger(tenant)
  const conversation = await seedConversation(tenant, {
    inboxId: page.inboxId,
    channel: "messenger",
    sourceId: uniqueDigits(),
  })
  return { tenant, api, page, conversation }
}

const SEND = new RegExp(graphPath("me/messages"))

describe("messages.create on Messenger", () => {
  it("sends the text to Graph with the page token and stores Meta's message id", async () => {
    const { api, page, conversation } = await setup("p111-mm-send")
    const mid = `m_${uniqueName("mid")}`
    await fake.script({
      match: { host: GRAPH, method: "POST", pathRegex: graphPath("me/messages"), bodyIncludes: conversation.sourceId },
      respond: { status: 200, json: { recipient_id: conversation.sourceId, message_id: mid } },
    })
    const since = await fake.cursor()
    const text = uniqueName("Hello Messenger")
    const sent = await api.post<Message>(messagesPath(conversation.conversationId), { text })
    expect(sent.status, sent.text).toBe(201)
    expect(sent.json).toMatchObject({ text, messageType: "outgoing", senderType: "api", type: "message", sourceId: null })

    const [call] = await waitForCalls({ host: GRAPH, since, path: SEND, method: "POST", bearer: page.accessToken })
    const body = parseBody<{
      recipient: { id: string }
      message: { text: string; metadata?: string }
      messaging_type: string
      tag?: string
    }>(call as NonNullable<typeof call>)
    expect(body.recipient).toEqual({ id: conversation.sourceId })
    expect(body.message.text).toBe(text)
    expect(typeof body.message.metadata).toBe("string")
    expect(body.messaging_type).toBe("RESPONSE")
    expect(body.tag).toBeUndefined()
    expect(keysOf(body)).toEqual(["message", "messaging_type", "recipient"])

    await wait(async () => {
      const one = await getMessage(api, conversation.conversationId, sent.json)
      expect(one.json.sourceId).toBe(mid)
    })
    expect(
      (await upstreamCalls({ host: GRAPH, since, path: SEND, bearer: page.accessToken })).length,
    ).toBe(1)
  })

  it("a Graph rejection marks the message failed (sendError) and emits messageFailed", async () => {
    const { tenant, api, conversation } = await setup("p111-mm-send-fail")
    await fake.script({
      match: { host: GRAPH, method: "POST", pathRegex: graphPath("me/messages"), bodyIncludes: conversation.sourceId },
      respond: { status: 400, json: graphError("(#100) The parameter recipient is invalid", 100) },
    })
    const socket = await memberSocket(tenant)
    try {
      const sent = await api.post<Message>(messagesPath(conversation.conversationId), { text: uniqueName("Will fail") })
      expect(sent.status, sent.text).toBe(201)
      const event = await socket.waitFor(
        "messageFailed",
        (e) => JSON.stringify(e.data).includes(sent.json.id),
        60_000,
      )
      expect(JSON.stringify(event.data)).toContain(sent.json.id)
      await wait(async () => {
        const one = await getMessage(api, conversation.conversationId, sent.json)
        expect(one.json.sendError).toEqual(expect.any(String))
        expect(one.json.sourceId).toBeNull()
      }, 60_000)
    } finally {
      await socket.close()
    }
  }, 90_000)
})

describe("comment replies on Messenger: create -> update -> changeAttributes -> delete", () => {
  it("replies, edits, likes/hides and deletes the comment at Graph; the row and realtime follow", async () => {
    const { tenant, api, page, conversation } = await setup("p111-mm-comment")
    const parentCommentId = `${page.pageId}_${uniqueDigits()}`
    const replyCommentId = `${page.pageId}_${uniqueDigits()}`
    const parent = await (await ext()).createMessage({
      workspaceId: tenant.workspaceId,
      conversationId: conversation.conversationId,
      contactInboxId: conversation.contactInboxId,
      messageType: "incoming",
      senderType: "contact",
      type: "comment",
      text: "Nice post",
      sourceId: parentCommentId,
    })
    await fake.script(
      {
        match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${reEscape(parentCommentId)}/comments`) },
        respond: { status: 200, json: { id: replyCommentId } },
      },
      {
        match: { host: GRAPH, method: "POST", pathRegex: graphPath(reEscape(replyCommentId)) },
        respond: { status: 200, json: { success: true } },
      },
      {
        match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${reEscape(replyCommentId)}/likes`) },
        respond: { status: 200, json: { success: true } },
      },
      {
        match: { host: GRAPH, method: "DELETE", pathRegex: graphPath(`${reEscape(replyCommentId)}/likes`) },
        respond: { status: 200, json: { success: true } },
      },
      {
        match: { host: GRAPH, method: "DELETE", pathRegex: graphPath(reEscape(replyCommentId)) },
        respond: { status: 200, json: { success: true } },
      },
    )
    const socket = await memberSocket(tenant)
    try {
      // 1. Reply to the comment.
      let since = await fake.cursor()
      const reply = await api.post<Message>(messagesPath(conversation.conversationId), {
        text: "Thanks!",
        replyToMessageId: parent.messageId,
        replyToMessageCreatedAt: parent.createdAt,
      })
      expect(reply.status, reply.text).toBe(201)
      expect(reply.json).toMatchObject({
        text: "Thanks!",
        type: "comment",
        parentId: parent.messageId,
        messageType: "outgoing",
        senderType: "api",
        sourceId: null,
      })
      const [replyCall] = await waitForCalls({
        host: GRAPH,
        since,
        path: new RegExp(graphPath(`${reEscape(parentCommentId)}/comments`)),
        bearer: page.accessToken,
      })
      expect(parseBody(replyCall as NonNullable<typeof replyCall>)).toEqual({ message: "Thanks!" })
      const assigned = await socket.waitFor(
        "messageIdAssigned",
        (e) => JSON.stringify(e.data).includes(reply.json.id),
        20_000,
      )
      expect(assigned.data).toEqual({ messageId: reply.json.id, commentId: replyCommentId })
      expect((await getMessage(api, conversation.conversationId, reply.json)).json.sourceId).toBe(replyCommentId)

      // 2. Edit it.
      since = await fake.cursor()
      const edited = await api.put(messagesPath(conversation.conversationId, `/${reply.json.id}`), {
        createdAt: reply.json.createdAt,
        newText: "  Thanks a lot!  ",
      })
      expect(edited.status, edited.text).toBe(200)
      expect(edited.json).toEqual({
        success: true,
        messageId: reply.json.id,
        newText: "Thanks a lot!",
        newAttachmentPath: null,
        newAttachmentPublicUrl: null,
        newAttachmentMimeType: null,
        newAttachmentWidth: 0,
        newAttachmentHeight: 0,
        removedAttachment: false,
      })
      const updated = await socket.waitFor(
        "messageUpdated",
        (e) => JSON.stringify(e.data).includes(reply.json.id),
        15_000,
      )
      expect(updated.data).toMatchObject({ messageId: reply.json.id, newText: "Thanks a lot!", removedAttachment: false })
      const [editCall] = await waitForCalls({
        host: GRAPH,
        since,
        path: new RegExp(graphPath(reEscape(replyCommentId))),
        method: "POST",
        bearer: page.accessToken,
      })
      expect(parseBody(editCall as NonNullable<typeof editCall>)).toEqual({ message: "Thanks a lot!" })
      expect((await getMessage(api, conversation.conversationId, reply.json)).json.text).toBe("Thanks a lot!")

      // 3. Like, then hide, then unlike.
      const attributesPath = messagesPath(conversation.conversationId, `/${reply.json.id}/attributes`)
      since = await fake.cursor()
      const liked = await api.post(attributesPath, { createdAt: reply.json.createdAt, liked: true })
      expect(liked.status, liked.text).toBe(200)
      expect(liked.json).toEqual({ success: true, messageId: reply.json.id })
      await waitForCalls({
        host: GRAPH,
        since,
        path: new RegExp(graphPath(`${reEscape(replyCommentId)}/likes`)),
        method: "POST",
        bearer: page.accessToken,
      })
      await wait(async () =>
        expect((await getMessage(api, conversation.conversationId, reply.json)).json.attributes).toEqual({
          liked: true,
          hidden: false,
        }),
      )

      since = await fake.cursor()
      expect((await api.post(attributesPath, { createdAt: reply.json.createdAt, hidden: true })).status).toBe(200)
      const [hideCall] = await waitForCalls({
        host: GRAPH,
        since,
        path: new RegExp(graphPath(reEscape(replyCommentId))),
        method: "POST",
        bearer: page.accessToken,
      })
      expect(parseBody(hideCall as NonNullable<typeof hideCall>)).toEqual({ is_hidden: true })
      await wait(async () =>
        expect((await getMessage(api, conversation.conversationId, reply.json)).json.attributes).toEqual({
          liked: true,
          hidden: true,
        }),
      )

      since = await fake.cursor()
      expect((await api.post(attributesPath, { createdAt: reply.json.createdAt, liked: false })).status).toBe(200)
      await waitForCalls({
        host: GRAPH,
        since,
        path: new RegExp(graphPath(`${reEscape(replyCommentId)}/likes`)),
        method: "DELETE",
        bearer: page.accessToken,
      })
      await wait(async () =>
        expect((await getMessage(api, conversation.conversationId, reply.json)).json.attributes).toEqual({
          liked: false,
          hidden: true,
        }),
      )

      // 4. Delete it.
      since = await fake.cursor()
      const deleted = await api.delete(messagesPath(conversation.conversationId, `/${reply.json.id}`), {
        json: { createdAt: reply.json.createdAt },
      })
      expect(deleted.status, deleted.text).toBe(204)
      const deletedEvent = await socket.waitFor(
        "messageDeleted",
        (e) => JSON.stringify(e.data).includes(reply.json.id),
        15_000,
      )
      expect(deletedEvent.data).toEqual({ messageIds: [reply.json.id] })
      await waitForCalls({
        host: GRAPH,
        since,
        path: new RegExp(graphPath(reEscape(replyCommentId))),
        method: "DELETE",
        bearer: page.accessToken,
      })
      expect((await getMessage(api, conversation.conversationId, reply.json)).json.deletedAt).toEqual(expect.any(String))
      // The parent comment is untouched.
      const parentRow = await getMessage(api, conversation.conversationId, { id: parent.messageId, createdAt: parent.createdAt })
      expect(parentRow.status, parentRow.text).toBe(200)
      expect(parentRow.json.deletedAt).toBeNull()
    } finally {
      await socket.close()
    }
  }, 120_000)

  it("an incoming comment cannot be edited (400) but can be liked at Graph", async () => {
    const { tenant, api, page, conversation } = await setup("p111-mm-comment-in")
    const commentId = `${page.pageId}_${uniqueDigits()}`
    const incoming = await (await ext()).createMessage({
      workspaceId: tenant.workspaceId,
      conversationId: conversation.conversationId,
      contactInboxId: conversation.contactInboxId,
      messageType: "incoming",
      senderType: "contact",
      type: "comment",
      text: "Great",
      sourceId: commentId,
    })
    expectSystemError(
      await api.put(messagesPath(conversation.conversationId, `/${incoming.messageId}`), {
        createdAt: incoming.createdAt,
        newText: "Changed",
      }),
      "Message is not an editable comment",
    )
    await fake.script({
      match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${reEscape(commentId)}/likes`) },
      respond: { status: 200, json: { success: true } },
    })
    const since = await fake.cursor()
    const liked = await api.post(messagesPath(conversation.conversationId, `/${incoming.messageId}/attributes`), {
      createdAt: incoming.createdAt,
      liked: true,
    })
    expect(liked.status, liked.text).toBe(200)
    await waitForCalls({
      host: GRAPH,
      since,
      path: new RegExp(graphPath(`${reEscape(commentId)}/likes`)),
      bearer: page.accessToken,
    })
    await wait(async () =>
      expect(
        (await getMessage(api, conversation.conversationId, { id: incoming.messageId, createdAt: incoming.createdAt })).json
          .attributes,
      ).toEqual({
        liked: true,
        hidden: false,
      }),
    )
  })

  it("more than one image on a Messenger comment reply -> 422 imagesNotSupportedOnChannel; nothing is stored", async () => {
    const { tenant, api, conversation } = await setup("p111-mm-comment-images")
    const parent = await (await ext()).createMessage({
      workspaceId: tenant.workspaceId,
      conversationId: conversation.conversationId,
      contactInboxId: conversation.contactInboxId,
      messageType: "incoming",
      senderType: "contact",
      type: "comment",
      text: "Show me",
      sourceId: uniqueDigits(),
    })
    const fileIds: string[] = []
    for (const name of ["a.png", "b.png"]) {
      const file = await api.post<{ id: string }>("/v1/media-library/files", {
        name,
        path: `public/space/${tenant.workspaceId}/media-library/${uniqueName("img")}`,
        mimeType: "image/png",
        size: 10,
      })
      expect(file.status, file.text).toBe(201)
      fileIds.push(file.json.id)
    }
    const response = await api.post(messagesPath(conversation.conversationId), {
      text: "Two pictures",
      mediaFileIds: fileIds,
      replyToMessageId: parent.messageId,
      replyToMessageCreatedAt: parent.createdAt,
    })
    const body = expectError(response, 422, "imagesNotSupportedOnChannel", {
      defined: true,
      message: "The messenger channel can send at most 1 image(s) here (reason=tooManyImages, channel=messenger, max=1)",
    })
    expect(body.data).toEqual({ reason: "tooManyImages", channel: "messenger", max: 1 })
    const page = await api.get<{ data: Message[] }>(messagesPath(conversation.conversationId))
    expect(page.json.data.map((message) => message.id)).toEqual([parent.messageId])
  })
})
