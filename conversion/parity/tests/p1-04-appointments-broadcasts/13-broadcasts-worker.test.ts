// One-off broadcasts that the worker sends: getAudience and listContacts after
// a send, resend, and stop / resume / delete while sending.

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  type Broadcast,
  createContact,
  createPublishedFlow,
  createWebchat,
  errorShape,
  expectError,
  newTenant,
} from "./helpers.ts"

const sendNow = async (api: Api, flowId: string, extra: Record<string, unknown> = {}): Promise<Broadcast> => {
  const response = await api.post<Broadcast>("/v1/broadcasts", {
    channel: "omnichannel",
    subaction: "allContacts",
    flowId,
    contactFilter: { operator: "and", conditions: [] },
    schedulesType: "now",
    schedulesAt: null,
    ...extra,
  })
  expect(response.status, response.text).toBe(201)
  expect(response.json).toMatchObject({ status: "scheduled", schedulesType: "now" })
  return response.json
}

const statusOf = async (api: Api, id: string): Promise<Broadcast> => {
  const response = await api.get<Broadcast>(`/v1/broadcasts/${id}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

// `finalizeBroadcasts` runs once a minute, so `sent` can take up to ~2 minutes.
const waitForStatus = (api: Api, id: string, statuses: string[], timeoutMs = 180_000) =>
  eventually(
    async () => {
      const broadcast = await statusOf(api, id)
      expect(statuses).toContain(broadcast.status)
      return broadcast
    },
    { timeoutMs, intervalMs: 500, description: `broadcast ${id} reaches ${statuses.join("/")}` },
  )

describe("a broadcast sent by the worker", () => {
  it(
    "delivers the flow to the audience; getAudience and listContacts show the recipient",
    async () => {
      const normalizer = new Normalizer()
      const { tenant, api } = await newTenant("bcw-send")
      const webchat = await createWebchat(api)
      const contact = await createContact(api, tenant, { inboxId: webchat.inboxId, firstName: "Ada" })
      const flowId = await createPublishedFlow(api, "Worker flow", "Hello from the parity broadcast")
      const created = await sendNow(api, flowId)

      const sent = await waitForStatus(api, created.id, ["sent"])
      expect(sent).toMatchObject({ id: created.id, status: "sent", contactCount: 1 })

      await eventually(
        async () => {
          const messages = await api.get(`/v1/conversations/${contact.conversationId}/messages`)
          expect(messages.text).toContain("Hello from the parity broadcast")
        },
        { timeoutMs: 30_000, intervalMs: 1_000, description: "broadcast message" },
      )

      const audience = await api.get<{ data: Array<Record<string, unknown>>; pageCount: number }>(
        `/v1/broadcasts/${created.id}/audience`,
      )
      expect(audience.status, audience.text).toBe(200)
      expect(audience.json.pageCount).toBe(1)
      expect(audience.json.data).toEqual([
        {
          contactId: contact.contactId,
          contact: expect.objectContaining({
            id: contact.contactId,
            firstName: "Ada",
            lastName: "Parity",
            phoneNumber: null,
            avatar: null,
            gender: "unknown",
          }),
          sent: true,
        },
      ])
      expect(normalizer.normalize(audience.json)).toMatchSnapshot("audience")

      const recipients = await eventually(
        async () => {
          const response = await api.get<{ data: Array<Record<string, unknown>>; total: number; pageCount: number }>(
            `/v1/broadcasts/${created.id}/contacts`,
            { query: { eventType: "message:sent" } },
          )
          expect(response.status, response.text).toBe(200)
          expect(response.json.total).toBe(1)
          return response.json
        },
        { timeoutMs: 30_000, intervalMs: 1_000, description: "message:sent recipients" },
      )
      expect(recipients.data).toEqual([
        expect.objectContaining({
          contactId: contact.contactId,
          firstName: "Ada",
          channel: "webchat",
          conversationId: contact.conversationId,
        }),
      ])
      // The webchat contact inbox source id is minted at random.
      expect(typeof recipients.data[0]?.sourceId).toBe("string")
      const stable = { ...recipients, data: recipients.data.map((row) => ({ ...row, sourceId: "<sourceId>" })) }
      expect(normalizer.normalize(stable)).toMatchSnapshot("message:sent recipients")
      const failed = await api.get(`/v1/broadcasts/${created.id}/contacts`, { query: { eventType: "message:failed" } })
      expect(failed.json).toEqual({ data: [], total: 0, pageCount: 0 })
    },
    300_000,
  )

  it(
    "resends a sent broadcast as a new one; refuses a broadcast that was not sent",
    async () => {
      const normalizer = new Normalizer()
      const { api } = await newTenant("bcw-resend")
      const flowId = await createPublishedFlow(api, "Resend flow")
      const created = await sendNow(api, flowId)
      const done = await waitForStatus(api, created.id, ["sent", "failed"])
      expect({ status: done.status, contactCount: done.contactCount }).toMatchSnapshot("empty audience outcome")

      const resent = await api.post<Broadcast>(`/v1/broadcasts/${created.id}/resend`, {})
      expect(resent.status, resent.text).toBe(201)
      expect(resent.json).toMatchObject({
        name: "Resend flow (Resend)",
        status: "scheduled",
        schedulesType: "now",
        flowId,
      })
      expect(resent.json.id).not.toBe(created.id)
      expect(normalizer.normalize(resent.json)).toMatchSnapshot("resent")
      await waitForStatus(api, resent.json.id, ["sent", "failed"])

      const draft = await api.post<Broadcast>("/v1/broadcasts", {
        channel: "omnichannel",
        subaction: "allContacts",
        flowId,
        contactFilter: { operator: "and", conditions: [] },
        schedulesType: "future",
        schedulesAt: "2030-01-07T09:00:00.000Z",
        saveAsDraft: true,
      })
      const notSent = await api.post(`/v1/broadcasts/${draft.json.id}/resend`, {})
      expect(expectError(notSent, 400, "systemError").message).toBe("Broadcast is not sent")
      expect(errorShape(notSent)).toMatchSnapshot("not sent")
      expect(errorShape(await api.post("/v1/broadcasts/1/resend", {}))).toMatchSnapshot("unknown")
    },
    300_000,
  )

  it(
    "stops and resumes a broadcast while it sends; a sending broadcast cannot be deleted",
    async () => {
      const { tenant, api } = await newTenant("bcw-stop")
      const webchat = await createWebchat(api)
      for (const firstName of ["Ada", "Bob", "Cy"]) {
        await createContact(api, tenant, { inboxId: webchat.inboxId, firstName })
      }
      const flowId = await createPublishedFlow(api, "Slow flow")
      // One recipient per minute keeps it `sending` for minutes.
      const created = await sendNow(api, flowId, { sendRatePerMinute: 1 })
      const sending = await waitForStatus(api, created.id, ["sending"])
      expect(sending.contactCount).toBe(3)
      expect(sending.sendRatePerMinute).toBe(1)

      const blocked = await api.delete(`/v1/broadcasts/${created.id}`)
      expect(expectError(blocked, 404, "notFound").message).toBe("Broadcast not found or cannot be deleted")

      const stopped = await api.post(`/v1/broadcasts/${created.id}/stop`, {})
      expect(stopped.status, stopped.text).toBe(200)
      expect(stopped.json).toEqual({ id: created.id })
      expect((await statusOf(api, created.id)).status).toBe("cancelled")

      const resumed = await api.post(`/v1/broadcasts/${created.id}/resume`, { sendRatePerMinute: 1 })
      expect(resumed.status, resumed.text).toBe(200)
      expect(resumed.json).toEqual({ id: created.id })
      expect((await statusOf(api, created.id)).status).toBe("sending")

      // Leave it stopped, then delete it.
      expect((await api.post(`/v1/broadcasts/${created.id}/stop`, {})).status).toBe(200)
      expect((await api.delete(`/v1/broadcasts/${created.id}`)).status).toBe(204)
    },
    300_000,
  )
})
