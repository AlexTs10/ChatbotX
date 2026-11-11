// channels.* — the API channel's inbound surface (`/v1/channels/api/*`),
// authenticated with the channel's own `cbx_api_` token from
// `connections.create {provider:"api"}`. sendMessage / markRead /
// deliveryStatus enqueue an integration job; the worker turns an inbound
// message into a contact + conversation (checked through `/v1/contacts`).

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { apiClient } from "../../src/http.ts"
import { uniqueId } from "../../src/unique.ts"
import { type Api, expectError, expectInvalid, tenantApi, uniqueName } from "./helpers.ts"

type Channel = { api: Api; channel: Api; inboxId: string; workspaceId: string; name: string }

const newChannel = async (label: string): Promise<Channel> => {
  const { tenant, api } = await tenantApi(label)
  const name = uniqueName("chan").slice(0, 40)
  const created = await api.post<{ connection: { inboxId: string }; secret: { token: string } }>("/v1/connections", {
    provider: "api",
    config: { name },
  })
  expect(created.status).toBe(201)
  return {
    api,
    channel: apiClient(created.json.secret.token),
    inboxId: created.json.connection.inboxId,
    workspaceId: tenant.workspaceId,
    name,
  }
}

const message = (overrides: { contact?: Record<string, unknown>; message?: Record<string, unknown> } = {}) => ({
  contact: { sourceId: `ext-${uniqueId()}`, firstName: "Ada", lastName: "Parity", ...overrides.contact },
  message: { sourceId: `msg-${uniqueId()}`, text: `hello ${uniqueId()}`, ...overrides.message },
})

describe("channels.me", () => {
  it("echoes the inbox and workspace of the token", async () => {
    const { channel, inboxId, workspaceId, name } = await newChannel("p105-ch-me")
    const response = await channel.get("/v1/channels/api/me")
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ inboxId, inboxName: name, workspaceId })
  })
})

describe("channels.sendMessage", () => {
  it(
    "accepts the message (202) and the worker creates the contact, its API inbox link and conversation",
    async () => {
      const { api, channel, inboxId } = await newChannel("p105-ch-send")
      const firstName = `Ada${uniqueId()}`
      const body = message({ contact: { firstName, email: "ada@parity.test", phoneNumber: "+15550001111" } })
      const response = await channel.post("/v1/channels/api/messages", body)
      expect(response.status).toBe(202)
      expect(response.json).toEqual({ accepted: true, messageSourceId: body.message.sourceId })

      const contact = await eventually(
        async () => {
          const list = await api.get<{ data: Array<Record<string, unknown>> }>("/v1/contacts")
          expect(list.status).toBe(200)
          const found = list.json.data.find((item) => item.firstName === firstName)
          expect(found).toBeDefined()
          return found as Record<string, unknown> & {
            contactInboxes: Array<Record<string, unknown>>
            conversation: Record<string, unknown>
          }
        },
        { timeoutMs: 30_000, intervalMs: 500, description: "contact from the API channel message" },
      )
      expect(contact).toMatchObject({
        firstName,
        lastName: "Parity",
        fullName: `${firstName} Parity`,
        email: "ada@parity.test",
        // Not asserted exactly: the original stored "+15550001111" in one run
        // and "+848864158" in another for this input (see spec P1.5 §8).
        phoneNumber: expect.stringMatching(/^\+\d+$/),
      })
      expect(contact.contactInboxes).toEqual([
        expect.objectContaining({
          inboxId,
          channel: "api",
          source: "inboundMessage",
          sourceId: body.contact.sourceId,
          lastIncomingMessageAt: expect.any(String),
        }),
      ])
      expect(contact.conversation).toMatchObject({
        contactId: contact.id,
        lastActivityAt: expect.any(String),
        contactRepliedAt: expect.any(String),
      })
    },
    90_000,
  )

  it("validates the payload (422)", async () => {
    const { channel } = await newChannel("p105-ch-send-bad")
    expectInvalid(await channel.post("/v1/channels/api/messages", {}), "contact")
    expectInvalid(
      await channel.post("/v1/channels/api/messages", message({ contact: { sourceId: "" } })),
      "contact.sourceId",
    )
    expectInvalid(
      await channel.post("/v1/channels/api/messages", message({ message: { sourceId: "" } })),
      "message.sourceId",
    )
    expectInvalid(
      await channel.post("/v1/channels/api/messages", message({ contact: { email: "not-an-email" } })),
      "contact.email",
    )
    expectInvalid(
      await channel.post("/v1/channels/api/messages", message({ contact: { avatar: "not a url" } })),
      "contact.avatar",
    )
    expectInvalid(
      await channel.post("/v1/channels/api/messages", message({ message: { contentType: "sticker" } })),
      "message.contentType",
    )
    expectInvalid(
      await channel.post(
        "/v1/channels/api/messages",
        message({ message: { attachments: [{ url: "nope", fileType: "image", mimeType: "image/png" }] } }),
      ),
      "message.attachments.0.url",
    )
    expectInvalid(
      await channel.post(
        "/v1/channels/api/messages",
        message({ message: { attachments: [{ url: "https://x.parity.test/a.png", fileType: "gif", mimeType: "image/gif" }] } }),
      ),
      "message.attachments.0.fileType",
    )
  })

  it("replays a request with the same Idempotency-Key", async () => {
    const { channel } = await newChannel("p105-ch-idem")
    const key = `idem-${uniqueId()}`
    const body = message()
    const first = await channel.post("/v1/channels/api/messages", body, { headers: { "idempotency-key": key } })
    expect(first.status).toBe(202)
    const replay = await channel.post("/v1/channels/api/messages", body, { headers: { "idempotency-key": key } })
    expect(replay.status).toBe(202)
    expect(replay.headers.get("idempotent-replayed")).toBe("true")
    expect(replay.json).toEqual(first.json)
    expectError(
      await channel.post("/v1/channels/api/messages", message(), { headers: { "idempotency-key": key } }),
      422,
      "idempotencyKeyReused",
    )
  })
})

describe("channels.typing / markRead / deliveryStatus", () => {
  it("typing answers 204 with no body and validates its input", async () => {
    const { channel } = await newChannel("p105-ch-typing")
    const response = await channel.post("/v1/channels/api/typing", { contact: { sourceId: "ext-1" }, typing: true })
    expect(response.status).toBe(204)
    expect(response.text).toBe("")
    expectInvalid(await channel.post("/v1/channels/api/typing", { contact: { sourceId: "ext-1" } }), "typing")
    expectInvalid(
      await channel.post("/v1/channels/api/typing", { contact: { sourceId: "" }, typing: false }),
      "contact.sourceId",
    )
  })

  it("markRead answers 204 and validates its input", async () => {
    const { channel } = await newChannel("p105-ch-read")
    const response = await channel.post("/v1/channels/api/read", { contact: { sourceId: `ext-${uniqueId()}` } })
    expect(response.status).toBe(204)
    expect(response.text).toBe("")
    expectInvalid(await channel.post("/v1/channels/api/read", {}), "contact")
  })

  it("deliveryStatus answers 204 and validates its input", async () => {
    const { channel } = await newChannel("p105-ch-status")
    for (const status of ["delivered", "read", "failed"]) {
      const response = await channel.post("/v1/channels/api/delivery-status", {
        messageId: `out-${uniqueId()}`,
        status,
        timestamp: new Date().toISOString(),
        ...(status === "failed" ? { error: { code: 42, message: "nope" } } : {}),
      })
      expect(response.status).toBe(204)
    }
    const base = { messageId: "m-1", status: "delivered", timestamp: new Date().toISOString() }
    expectInvalid(await channel.post("/v1/channels/api/delivery-status", { ...base, status: "lost" }), "status")
    expectInvalid(await channel.post("/v1/channels/api/delivery-status", { ...base, messageId: "" }), "messageId")
    const { timestamp: _t, ...noTimestamp } = base
    expectInvalid(await channel.post("/v1/channels/api/delivery-status", noTimestamp), "timestamp")
  })
})

describe("channels rate limit", () => {
  it("allows 120 requests per inbox per 10 s window, then 429 tooManyRequests with Retry-After", async () => {
    const { channel } = await newChannel("p105-ch-rate")
    const responses = await Promise.all(
      Array.from({ length: 250 }, () =>
        channel.post("/v1/channels/api/typing", { contact: { sourceId: "ext-rate" }, typing: true }),
      ),
    )
    const statuses = responses.map((response) => response.status)
    expect(statuses.every((status) => status === 204 || status === 429)).toBe(true)
    // 250 requests span at most two windows, so at least 10 are refused.
    expect(statuses.filter((status) => status === 429).length).toBeGreaterThanOrEqual(10)
    expect(statuses.filter((status) => status === 204).length).toBeLessThanOrEqual(240)
    const limited = responses.find((response) => response.status === 429)
    expectError(limited as (typeof responses)[number], 429, "tooManyRequests")
    expect(limited?.headers.get("retry-after")).toMatch(/^\d+$/)
  })
})
