// Keyword automations firing on an inbound contact message (API channel).
// An inbound message is queued for Keywords -> AI; after the automated-response
// delay the worker matches the enabled inbound rules (oldest first) and the
// first match either sends its text (`sendChatMessage`) or starts its flow
// (`sendFlow`). The observable effect is the API channel's signed outbound
// `message_created` callback on the fake upstream and the realtime
// `messageCreated` event. Rule changes apply at once (the rule cache is
// invalidated on every write).

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  type Api,
  type ApiChannel,
  type ApiContact,
  type CallbackEnvelope,
  callbacks,
  callbackTexts,
  createApiChannel,
  createPublishedFlow,
  inboundContact,
  memberSocket,
  newTenant,
  parseBody,
  sendInbound,
  uniqueName,
  wait,
} from "./helpers.ts"

const createKeyword = async (api: Api, body: Record<string, unknown>): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/keywords", body)
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

const setup = async (label: string) => {
  const { tenant, api } = await newTenant(label)
  const channel = await createApiChannel(api)
  const contact = await inboundContact(api, channel)
  return { tenant, api, channel, contact }
}

/** Waits until the channel delivered exactly `expected` (in order) since `since`. */
const expectReplies = (channel: ApiChannel, since: number, expected: string[]) =>
  wait(async () => {
    expect(await callbackTexts(channel, since)).toEqual(expected)
  })

const say = (channel: ApiChannel, contact: ApiContact, text: string) => sendInbound(channel, contact, text)

describe("keywords fire on inbound messages", () => {
  it("a text rule replies through the channel, signed, as an outgoing message (also on realtime)", async () => {
    const { tenant, api, channel, contact } = await setup("p110-kwt-text")
    const reply = `Our price is ${uniqueId()}`
    await createKeyword(api, { keywords: ["price"], text: reply })
    const { socket } = await memberSocket(tenant)
    try {
      const since = await fake.cursor()
      await say(channel, contact, "What is the PRICE please?")
      await expectReplies(channel, since, [reply])
      const [record] = await callbacks(channel, since)
      const body = parseBody<CallbackEnvelope>(record as NonNullable<typeof record>)
      expect(body).toMatchObject({
        event: "message_created",
        contact: { sourceId: contact.sourceId },
        message: { text: reply, messageType: "outgoing", contentType: "text" },
      })
      expect(record?.headers["x-chatbotx-signature"]).toMatch(/^sha256=[0-9a-f]{64}$/)
      const event = await socket.waitFor("messageCreated", (e) => JSON.stringify(e.data).includes(reply), 20_000)
      expect(event.data).toMatchObject({ text: reply, messageType: "outgoing" })
    } finally {
      await socket.close()
    }
  })

  it("a flow rule starts its published flow", async () => {
    const { api, channel, contact } = await setup("p110-kwt-flow")
    const first = `Flow says hi ${uniqueId()}`
    const second = `Flow says bye ${uniqueId()}`
    const flowId = await createPublishedFlow(api, uniqueName("KwFlow"), first, second)
    await createKeyword(api, { keywords: ["start"], flowId })
    const since = await fake.cursor()
    await say(channel, contact, "please start")
    await expectReplies(channel, since, [first, second])
  })

  it("matching ignores case and Vietnamese accents (`contains`)", async () => {
    const { api, channel, contact } = await setup("p110-kwt-accents")
    const reply = `Advice ${uniqueId()}`
    await createKeyword(api, { conditions: [{ operator: "contains", values: ["tu van"] }], text: reply })
    const since = await fake.cursor()
    await say(channel, contact, "Cần TƯ VẤN giúp tôi")
    await expectReplies(channel, since, [reply])
  })

  it("a disabled rule and a non-matching exact rule stay silent; the matching message replies once", async () => {
    const { api, channel, contact } = await setup("p110-kwt-silent")
    const disabledReply = `Disabled ${uniqueId()}`
    const exactReply = `Exact ${uniqueId()}`
    const disabledId = await createKeyword(api, { keywords: ["alpha"], text: disabledReply })
    const off = await api.patch(`/v1/keywords/${disabledId}/status`, { status: false })
    expect(off.status, off.text).toBe(200)
    await createKeyword(api, { conditions: [{ operator: "is", values: ["beta"] }], text: exactReply })
    const since = await fake.cursor()
    await say(channel, contact, "alpha")
    await say(channel, contact, "beta please")
    await say(channel, contact, "Beta")
    await expectReplies(channel, since, [exactReply])
    expect(await callbackTexts(channel, since)).not.toContain(disabledReply)
  })

  it("all conditions must match; notContains blocks the rule", async () => {
    const { api, channel, contact } = await setup("p110-kwt-and")
    const reply = `Order help ${uniqueId()}`
    await createKeyword(api, {
      conditions: [
        { operator: "containsWholeWord", values: ["order"] },
        { operator: "notContains", values: ["cancel"] },
      ],
      text: reply,
    })
    const since = await fake.cursor()
    await say(channel, contact, "cancel my order")
    await say(channel, contact, "reorder now")
    await say(channel, contact, "where is my order?")
    await expectReplies(channel, since, [reply])
  })

  it("when two rules match, the older one replies", async () => {
    const { api, channel, contact } = await setup("p110-kwt-order")
    const older = `Older ${uniqueId()}`
    const newer = `Newer ${uniqueId()}`
    await createKeyword(api, { keywords: ["promo"], text: older })
    await createKeyword(api, { keywords: ["promo"], text: newer })
    const since = await fake.cursor()
    await say(channel, contact, "any promo today?")
    await expectReplies(channel, since, [older])
  })

  it("an update applies to the next message; a deleted rule no longer fires", async () => {
    const { api, channel, contact } = await setup("p110-kwt-update")
    const before = `Before ${uniqueId()}`
    const after = `After ${uniqueId()}`
    const sentinel = `Sentinel ${uniqueId()}`
    const id = await createKeyword(api, { keywords: ["hours"], text: before })
    const updated = await api.patch(`/v1/keywords/${id}`, { text: after })
    expect(updated.status, updated.text).toBe(200)
    let since = await fake.cursor()
    await say(channel, contact, "opening hours?")
    await expectReplies(channel, since, [after])

    expect((await api.delete(`/v1/keywords/${id}`)).status).toBe(204)
    await createKeyword(api, { keywords: ["sentinel"], text: sentinel })
    since = await fake.cursor()
    await say(channel, contact, "opening hours?")
    await say(channel, contact, "sentinel")
    await expectReplies(channel, since, [sentinel])
  })

  it("an outbound rule does not fire on a contact message", async () => {
    const { api, channel, contact } = await setup("p110-kwt-outbound")
    const outboundReply = `Outbound ${uniqueId()}`
    const sentinel = `Sentinel ${uniqueId()}`
    await createKeyword(api, { type: "outbound", keywords: ["thanks"], text: outboundReply })
    await createKeyword(api, { keywords: ["sentinel"], text: sentinel })
    const since = await fake.cursor()
    await say(channel, contact, "thanks")
    await say(channel, contact, "sentinel")
    await expectReplies(channel, since, [sentinel])
  })
})
