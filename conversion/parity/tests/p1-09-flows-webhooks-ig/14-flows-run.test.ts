// A published flow runs for a contact. Setup: an API channel whose outbound
// callback is a fake-upstream host, a contact made by an inbound API-channel
// message, and `contacts.sendFlow` (`POST /v1/contacts/id:<id>/flows`) to
// start the flow. Observed effects: the signed `message_created` callbacks
// (one per send step, in flow order), the published version is what runs
// (not a newer draft), the action step's tag on the contact, the stored
// outgoing messages and the realtime `messageCreated` events.

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { fake } from "../../src/fake.ts"
import {
  callbacks,
  createApiChannel,
  createPublishedFlow,
  expectError,
  inboundContact,
  memberSocket,
  newTenant,
  parseBody,
  textSpec,
  uniqueName,
} from "./helpers.ts"

const wait = <T>(fn: () => Promise<T>) => eventually(fn, { timeoutMs: 45_000, intervalMs: 500 })

type Envelope = {
  event: string
  timestamp: string
  contact: { id: string; sourceId: string }
  message: { text: string; messageType: string; contentType: string; [key: string]: unknown }
}

describe("a published flow runs for a contact (API channel)", () => {
  it("sends each step to the channel callback in order, signed; the action step tags the contact", async () => {
    const { tenant, api } = await newTenant("p109-run")
    const channel = await createApiChannel(api)
    const contact = await inboundContact(api, channel, wait)
    const { socket } = await memberSocket(tenant)
    try {
      const name = uniqueName("Run")
      const tag = uniqueName("p109-tag")
      const first = `Hello from ${name}`
      const last = `Bye from ${name}`
      const flowId = await createPublishedFlow(api, {
        formatVersion: 1,
        name,
        steps: [
          { type: "send", text: first },
          { type: "action", action: "addTags", tagNames: [tag] },
          { type: "send", text: last },
        ],
      })
      // A newer draft is not what runs.
      expect((await api.put(`/v1/flows/${flowId}/draft`, { spec: textSpec(name, "draft text") })).status).toBe(204)

      const since = await fake.cursor()
      const started = await api.post(`/v1/contacts/id:${contact.contactId}/flows`, { flowId })
      expect(started.status, started.text).toBe(204)

      const records = await wait(async () => {
        const found = await callbacks(channel, since)
        expect(found.map((record) => parseBody<Envelope>(record).message.text)).toEqual([first, last])
        return found
      })
      for (const record of records) {
        const body = parseBody<Envelope>(record)
        expect(body).toMatchObject({
          event: "message_created",
          contact: { id: expect.stringMatching(/^\d+$/), sourceId: contact.sourceId },
          message: { messageType: "outgoing", contentType: "text" },
        })
        expect(Date.parse(body.timestamp)).not.toBeNaN()
        expect(record.headers["content-type"]).toMatch(/^application\/json/)
        expect(record.headers["x-chatbotx-signature"]).toMatch(/^sha256=[0-9a-f]{64}$/)
        expect(record.headers["x-chatbotx-timestamp"]).toMatch(/^\d{10}$/)
        expect(record.headers["x-chatbotx-delivery"]).toMatch(/^[0-9a-f-]{36}$/)
      }
      expect(records.some((record) => record.bodyText.includes("draft text"))).toBe(false)

      // The action step added the tag (created by name).
      await wait(async () => {
        const tags = await api.get<{ data: Array<{ name: string }> }>(`/v1/contacts/id:${contact.contactId}/tags`)
        expect(tags.status, tags.text).toBe(200)
        expect(tags.json.data.map((item) => item.name)).toContain(tag)
      })

      // The sent messages are stored as outgoing messages of the conversation.
      await wait(async () => {
        const messages = await api.get<{ data: Array<{ text: string | null; messageType: string }> }>(
          `/v1/contacts/id:${contact.contactId}/messages`,
        )
        expect(messages.status, messages.text).toBe(200)
        const outgoing = messages.json.data.filter((message) => message.messageType === "outgoing").map((m) => m.text)
        expect(outgoing).toEqual(expect.arrayContaining([first, last]))
        expect(outgoing).not.toContain("draft text")
      })

      // Realtime: members see the flow's messages.
      for (const text of [first, last]) {
        const event = await socket.waitFor("messageCreated", (e) => JSON.stringify(e.data).includes(text), 20_000)
        expect(event.data).toMatchObject({ text, messageType: "outgoing" })
      }
    } finally {
      await socket.close()
    }
  })

  it("a re-published flow runs its new version", async () => {
    const { api } = await newTenant("p109-run-republish")
    const channel = await createApiChannel(api)
    const contact = await inboundContact(api, channel, wait)
    const name = uniqueName("Again")
    const flowId = await createPublishedFlow(api, textSpec(name, `v1 ${name}`))
    const republished = await api.post(`/v1/flows/${flowId}/publish`, { spec: textSpec(name, `v2 ${name}`) })
    expect(republished.status, republished.text).toBe(204)
    const since = await fake.cursor()
    expect((await api.post(`/v1/contacts/id:${contact.contactId}/flows`, { flowId })).status).toBe(204)
    await wait(async () => {
      const found = await callbacks(channel, since)
      expect(found.map((record) => parseBody<Envelope>(record).message.text)).toEqual([`v2 ${name}`])
    })
  })

  it("a deleted flow can no longer be sent (404)", async () => {
    const { api } = await newTenant("p109-run-deleted")
    const channel = await createApiChannel(api)
    const contact = await inboundContact(api, channel, wait)
    const flowId = await createPublishedFlow(api, textSpec(uniqueName("Gone"), "never sent"))
    expect((await api.delete(`/v1/flows/${flowId}`)).status).toBe(204)
    const response = await api.post(`/v1/contacts/id:${contact.contactId}/flows`, { flowId })
    expectError(response, 404, "notFound")
  })
})
