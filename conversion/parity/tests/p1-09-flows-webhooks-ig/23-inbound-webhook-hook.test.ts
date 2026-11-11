// The public receiving endpoint `POST /api/hooks/<token>` (no API token; the
// token in the path is the credential): answers, error codes, idempotency,
// size and rate limits, and the request counter.

import { beforeAll, describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { HttpClient } from "../../src/http.ts"
import { comparableHeaders } from "../../src/normalize.ts"
import {
  type Api,
  activeWebhook,
  configure,
  createApiChannel,
  createWebhook,
  deliver,
  deliverAccepted,
  hookClient,
  hookPath,
  setActive,
  SNOWFLAKE,
  tenantApi,
  type Webhook,
  type WebhookConfig,
  type WebhookRequest,
  type WebhookSummary,
  waitForRequest,
} from "./helpers-webhooks.ts"

const apiConfig = (inboxId: string): WebhookConfig => ({
  inboxResolution: { mode: "inbox", inboxId },
  identifiers: [{ kind: "channelUserId", jsonPath: "$.contact.id" }],
  fieldMappings: [{ jsonPath: "$.contact.first", target: "first_name" }],
  actions: [],
})

const failure = (code: string, message: string) => ({ accepted: false, code, message })

let api: Api
let inboxId: string
let webhook: Webhook

beforeAll(async () => {
  ;({ api } = await tenantApi("p109-hook"))
  ;({ inboxId } = await createApiChannel(api))
  webhook = await activeWebhook(api, apiConfig(inboxId))
})

describe("POST /api/hooks/<token>", () => {
  it("accepts a JSON object (202 {accepted, requestId}, Cache-Control no-store)", async () => {
    const response = await deliver(webhook.endpointUrl as string, { contact: { id: "hook-1", first: "Ada" } })
    expect(response.status, response.text).toBe(202)
    expect(response.json).toEqual({ accepted: true, requestId: expect.stringMatching(SNOWFLAKE) })
    expect(comparableHeaders(response.headers)).toMatchObject({
      "content-type": "application/json",
      "cache-control": "no-store",
    })
    const request = await waitForRequest(api, webhook.id, response.json.requestId as string)
    expect(request.status).toBe("processed")
  })

  it("accepts a JSON array body and any content type", async () => {
    const http = hookClient()
    const response = await http.request("POST", hookPath(webhook.endpointUrl as string), {
      body: JSON.stringify([{ contact: { id: "hook-array" } }]),
      headers: { "content-type": "text/plain" },
    })
    expect(response.status, response.text).toBe(202)
    expect(response.json).toMatchObject({ accepted: true })
  })

  it("refuses a body that is not a JSON object or array (400 INVALID_JSON)", async () => {
    const url = hookPath(webhook.endpointUrl as string)
    for (const body of ["not json", "42", '"text"', "null", "", '{"a":"x\\u0000y"}']) {
      const response = await hookClient().request("POST", url, { body, headers: { "content-type": "application/json" } })
      expect({ body, status: response.status }).toEqual({ body, status: 400 })
      expect(response.json).toEqual(failure("INVALID_JSON", "The body must be a JSON object or array"))
    }
  })

  it("refuses a body over 256 KiB (413 PAYLOAD_TOO_LARGE), even for an unknown token", async () => {
    const big = JSON.stringify({ blob: "x".repeat(256 * 1024) })
    const known = await hookClient().request("POST", hookPath(webhook.endpointUrl as string), {
      body: big,
      headers: { "content-type": "application/json" },
    })
    expect(known.status).toBe(413)
    expect(known.json).toEqual(failure("PAYLOAD_TOO_LARGE", "The request body is too large"))
    const unknown = await hookClient().request("POST", `/api/hooks/iwh_${"a".repeat(43)}`, {
      body: big,
      headers: { "content-type": "application/json" },
    })
    expect(unknown.status).toBe(413)
    // Just at the limit is accepted.
    const exact = JSON.stringify({ blob: "x".repeat(256 * 1024 - 11) })
    expect(Buffer.byteLength(exact)).toBe(256 * 1024)
    const ok = await hookClient().request("POST", hookPath(webhook.endpointUrl as string), {
      body: exact,
      headers: { "content-type": "application/json" },
    })
    expect(ok.status, ok.text).toBe(202)
  })

  it("404 NOT_FOUND for a malformed or unknown token", async () => {
    for (const token of ["nope", "iwh_short", `iwh_${"a".repeat(65)}`, `iwx_${"a".repeat(43)}`, `iwh_${"b".repeat(43)}`]) {
      const response = await hookClient().post(`/api/hooks/${token}`, { contact: { id: "x" } })
      expect({ token, status: response.status }).toEqual({ token, status: 404 })
      expect(response.json).toEqual(failure("NOT_FOUND", "Webhook not found"))
      expect(comparableHeaders(response.headers)["cache-control"]).toBe("no-store")
    }
  })

  it("405 with Allow: POST for other methods", async () => {
    const path = hookPath(webhook.endpointUrl as string)
    for (const method of ["GET", "PUT", "PATCH", "DELETE", "OPTIONS"]) {
      const response = await hookClient().request(method, path)
      expect({ method, status: response.status }).toEqual({ method, status: 405 })
      expect(response.text).toBe("")
      expect(comparableHeaders(response.headers)).toMatchObject({ allow: "POST", "cache-control": "no-store" })
    }
  })

  it("403 WEBHOOK_INACTIVE for an unconfigured or deactivated webhook; accepts again once active", async () => {
    const created = await createWebhook(api)
    const url = created.endpointUrl as string
    const inactive = failure("WEBHOOK_INACTIVE", "This webhook is not active")
    const unconfigured = await deliver(url, { contact: { id: "x" } })
    expect(unconfigured.status).toBe(403)
    expect(unconfigured.json).toEqual(inactive)

    // Configured but still inactive.
    await configure(api, created.id, apiConfig(inboxId))
    await eventually(
      async () => {
        const response = await deliver(url, { contact: { id: "x" } })
        expect(response.status).toBe(403)
        expect(response.json).toEqual(inactive)
      },
      { timeoutMs: 15_000, intervalMs: 500 },
    )
    await setActive(api, created.id, true)
    await eventually(async () => expect((await deliver(url, { contact: { id: "x" } })).status).toBe(202), {
      timeoutMs: 15_000,
      intervalMs: 500,
      description: "activated webhook accepts",
    })
    await setActive(api, created.id, false)
    await eventually(
      async () => {
        const response = await deliver(url, { contact: { id: "x" } })
        expect(response.status).toBe(403)
        expect(response.json).toEqual(inactive)
      },
      { timeoutMs: 15_000, intervalMs: 500, description: "deactivated webhook refuses" },
    )
  })

  it("dedupes by Idempotency-Key per webhook (200 {accepted, duplicate}); the counter moves once", async () => {
    const own = await activeWebhook(api, apiConfig(inboxId))
    const url = own.endpointUrl as string
    const key = `key-${Date.now()}`
    const first = await deliver(url, { contact: { id: "idem" } }, { headers: { "idempotency-key": `  ${key}  ` } })
    expect(first.status, first.text).toBe(202)
    const requestId = first.json.requestId as string
    // Same key (trimmed), even with another body: a duplicate, not a new request.
    const again = await deliver(url, { contact: { id: "other" } }, { headers: { "idempotency-key": key } })
    expect(again.status, again.text).toBe(200)
    expect(again.json).toEqual({ accepted: true, duplicate: true })
    // No key: every delivery is new, even with the same body.
    const a = await deliverAccepted(url, { contact: { id: "same" } })
    const b = await deliverAccepted(url, { contact: { id: "same" } })
    expect(new Set([requestId, a, b]).size).toBe(3)
    // The key is per webhook.
    const otherHook = await activeWebhook(api, apiConfig(inboxId))
    expect((await deliver(otherHook.endpointUrl as string, {}, { headers: { "idempotency-key": key } })).status).toBe(202)

    const list = await api.get<{ data: WebhookSummary[] }>("/v1/inbound-webhooks")
    const summary = list.json.data.find((item) => item.id === own.id)
    expect(summary).toMatchObject({ requestCount: 3, updatedAt: own.updatedAt })
    const requests = await api.get<{ data: WebhookRequest[]; pageCount: number }>(`/v1/inbound-webhooks/${own.id}/requests`)
    expect(requests.json.data.map((request) => request.id).sort()).toEqual([requestId, a, b].sort())
  })

  it("refuses an Idempotency-Key over 255 characters (400 INVALID_IDEMPOTENCY_KEY); 255 is fine", async () => {
    const url = webhook.endpointUrl as string
    const long = await deliver(url, { contact: { id: "k" } }, { headers: { "idempotency-key": "k".repeat(256) } })
    expect(long.status).toBe(400)
    expect(long.json).toEqual(failure("INVALID_IDEMPOTENCY_KEY", "Idempotency-Key is too long"))
    const max = await deliver(url, { contact: { id: "k" } }, { headers: { "idempotency-key": `${Date.now()}`.padEnd(255, "k") } })
    expect(max.status, max.text).toBe(202)
    // A blank key counts as no key.
    expect((await deliver(url, { contact: { id: "k" } }, { headers: { "idempotency-key": "   " } })).status).toBe(202)
    expect((await deliver(url, { contact: { id: "k" } }, { headers: { "idempotency-key": "   " } })).status).toBe(202)
  })

  it(
    "limits each client address to 300 requests per 10 s window before the token lookup (429 RATE_LIMITED, Retry-After)",
    async () => {
      const client = new HttpClient()
      const path = `/api/hooks/iwh_${"r".repeat(43)}`
      const statuses: number[] = []
      let limited: Awaited<ReturnType<HttpClient["post"]>> | undefined
      // 700 requests in < 10 s span at most two fixed windows, so one window
      // gets more than 300 and the first 300 of the first window pass.
      for (let batch = 0; batch < 14; batch += 1) {
        const responses = await Promise.all(
          Array.from({ length: 50 }, () => client.post<Record<string, unknown>>(path, { a: 1 })),
        )
        for (const response of responses) {
          statuses.push(response.status)
          if (response.status === 429 && !limited) {
            limited = response
          }
        }
      }
      expect(statuses.filter((status) => status === 404).length).toBeGreaterThanOrEqual(300)
      expect(statuses.filter((status) => status !== 404 && status !== 429)).toEqual([])
      expect(limited, "no 429 seen").toBeDefined()
      expect(limited?.json).toEqual(failure("RATE_LIMITED", "Too many requests"))
      const retryAfter = Number(limited?.headers.get("retry-after"))
      expect(retryAfter).toBeGreaterThanOrEqual(1)
      expect(retryAfter).toBeLessThanOrEqual(10)
      // Another client address is not limited.
      expect((await hookClient().post(path, { a: 1 })).status).toBe(404)
    },
    60_000,
  )
})
