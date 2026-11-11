// externalWebhooks.* — URLs an automation platform (Make, n8n) registers for
// an event name. Registration checks the URL with the SSRF guard (DoH lookup
// on 1.1.1.1, answered by the fake upstream). A flow "make" step POSTs the
// contact to every URL registered for its events (worker, SSRF-guarded
// external request); that delivery is checked here as the registration's
// effect.

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { fake } from "../../src/fake.ts"
import { uniqueHost } from "../../src/unique.ts"
import {
  type Api,
  dohLookups,
  expectError,
  expectInvalid,
  keysOf,
  qs,
  tenantApi,
  uniqueName,
  upstreamCalls,
} from "./helpers.ts"

type Webhook = { id: string; provider: string; event: string; url: string }

const register = async (api: Api, body: Record<string, unknown>): Promise<Webhook> => {
  const response = await api.post<Webhook>("/v1/external-webhooks", body)
  expect(response.status, response.text).toBe(201)
  return response.json
}

const list = async (api: Api, query: Record<string, number> = {}) => {
  const response = await api.get<{ data: Webhook[]; pageCount: number }>(`/v1/external-webhooks${qs(query)}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("externalWebhooks.create", () => {
  it("registers a URL after a DoH check of its host; registering it again returns the same row", async () => {
    const { api } = await tenantApi("p108-ew-create")
    const host = uniqueHost("hook")
    const url = `https://${host}/make/abc?x=1`
    const since = await fake.cursor()

    const created = await register(api, { url: `  ${url}  `, event: " new_order " })
    expect(keysOf(created)).toEqual(["event", "id", "provider", "url"])
    expect(created).toMatchObject({ provider: "make", event: "new_order", url })
    expect(created.id).toMatch(/^\d{15,20}$/)

    const doh = await dohLookups(host, since)
    expect(doh.length).toBeGreaterThanOrEqual(1)
    expect(doh[0]).toMatchObject({ scheme: "https", method: "GET", query: { name: host, type: "A" } })
    // Registration makes no call to the URL itself.
    expect(await upstreamCalls({ host, since })).toHaveLength(0)

    const again = await register(api, { url, event: "new_order", provider: "n8n" })
    expect(again).toEqual(created)

    const n8n = await register(api, { url, event: "other_event", provider: "n8n" })
    expect(n8n).toMatchObject({ provider: "n8n", event: "other_event", url })
    expect(n8n.id).not.toBe(created.id)
  })

  it("refuses a URL the SSRF guard blocks (422 invalidRequestData, defined false, with the guard's message)", async () => {
    const { api } = await tenantApi("p108-ew-ssrf")
    const host = uniqueHost("hook-private")
    await fake.dns({ name: host, a: ["192.168.10.20"] })
    const blocked = [`https://${host}/x`, "http://localhost:8080/x", "http://10.1.2.3/x", "http://[::1]/x", "ftp://example.com/x"]
    for (const url of blocked) {
      // `defined: false`: thrown by the service, not by oRPC input validation.
      expectError(await api.post("/v1/external-webhooks", { url, event: "e" }), 422, "invalidRequestData", {
        defined: false,
        message: `[ssrf-guard] Webhook URL is not allowed: ${url}`,
      })
    }
    expect((await list(api)).data).toEqual([])
  })

  it("validates the input (422)", async () => {
    const { api } = await tenantApi("p108-ew-invalid")
    expectInvalid(await api.post("/v1/external-webhooks", { event: "e" }), "url")
    expectInvalid(await api.post("/v1/external-webhooks", { url: "not a url", event: "e" }), "url")
    expectInvalid(await api.post("/v1/external-webhooks", { url: "https://example.com" }), "event")
    expectInvalid(await api.post("/v1/external-webhooks", { url: "https://example.com", event: "   " }), "event")
    expectInvalid(await api.post("/v1/external-webhooks", { url: "https://example.com", event: "e".repeat(101) }), "event")
    expectInvalid(
      await api.post("/v1/external-webhooks", { url: "https://example.com", event: "e", provider: "zapier" }),
      "provider",
    )
  })

  it("allows 50 webhooks per workspace (400 externalWebhookLimitReached after that)", async () => {
    const { api } = await tenantApi("p108-ew-limit")
    const host = uniqueHost("hook-limit")
    for (let index = 0; index < 50; index++) {
      await register(api, { url: `https://${host}/${index}`, event: "e" })
    }
    expectError(await api.post("/v1/external-webhooks", { url: `https://${host}/50`, event: "e" }), 400, "externalWebhookLimitReached", {
      defined: false,
      message: "Workspace has reached the maximum of 50 external webhooks",
    })
    // An existing (event, url) is still returned.
    expect((await api.post("/v1/external-webhooks", { url: `https://${host}/0`, event: "e" })).status).toBe(201)
    expect((await list(api, { perPage: 50 })).pageCount).toBe(1)
  })
})

describe("externalWebhooks.list", () => {
  it("lists the workspace's registrations, paged in memory (pageCount is at least 1)", async () => {
    const { api } = await tenantApi("p108-ew-list")
    const { api: other } = await tenantApi("p108-ew-list-other")
    expect(await list(api)).toEqual({ data: [], pageCount: 1 })

    const host = uniqueHost("hook-list")
    const a = await register(api, { url: `https://${host}/a`, event: "one" })
    const b = await register(api, { url: `https://${host}/b`, event: "two", provider: "n8n" })
    const c = await register(api, { url: `https://${host}/c`, event: "three" })
    await register(other, { url: `https://${host}/a`, event: "one" })

    const all = await list(api)
    expect(all.pageCount).toBe(1)
    const byId = (rows: Webhook[]) => [...rows].sort((x, y) => x.id.localeCompare(y.id))
    expect(byId(all.data)).toEqual(byId([a, b, c]))

    const page1 = await list(api, { perPage: 2, page: 1 })
    const page2 = await list(api, { perPage: 2, page: 2 })
    expect(page1.pageCount).toBe(2)
    expect(page1.data).toHaveLength(2)
    expect(page2.data).toHaveLength(1)
    expect(byId([...page1.data, ...page2.data])).toEqual(byId([a, b, c]))
    expect((await list(api, { perPage: 2, page: 3 })).data).toEqual([])
  })
})

describe("externalWebhooks.delete", () => {
  it("unregisters a webhook (204); 404 for an unknown, foreign or deleted id", async () => {
    const { api } = await tenantApi("p108-ew-delete")
    const { api: other } = await tenantApi("p108-ew-delete-other")
    const host = uniqueHost("hook-del")
    const mine = await register(api, { url: `https://${host}/mine`, event: "e" })
    const foreign = await register(other, { url: `https://${host}/theirs`, event: "e" })

    const deleted = await api.delete(`/v1/external-webhooks/${mine.id}`)
    expect(deleted.status).toBe(204)
    expect(deleted.text).toBe("")
    expect((await list(api)).data).toEqual([])
    expectError(await api.delete(`/v1/external-webhooks/${mine.id}`), 404, "notFound", {
      defined: true,
      message: "External webhook not found",
    })
    expectError(await api.delete(`/v1/external-webhooks/${foreign.id}`), 404, "notFound")
    expect((await list(other)).data).toEqual([foreign])
    expectInvalid(await api.delete("/v1/external-webhooks/abc"), "id")
  })
})

describe("external webhook delivery (flow make step)", () => {
  it("POSTs the contact to every URL registered for the step's events", async () => {
    const { api } = await tenantApi("p108-ew-deliver")
    const host = uniqueHost("hook-deliver")
    await fake.script(
      { match: { host, method: "POST", path: "/make" }, respond: { status: 200, json: { accepted: true } } },
      { match: { host, method: "POST", path: "/n8n" }, respond: { status: 200, json: { ok: true } } },
    )
    await register(api, { url: `https://${host}/make`, event: "lead_ready" })
    await register(api, { url: `https://${host}/n8n`, event: "lead_ready", provider: "n8n" })
    // Another event: not called.
    await register(api, { url: `https://${host}/unused`, event: "other" })

    const webchat = await api.post<{ inboxId: string }>("/v1/webchats", {
      name: uniqueName("Hook chat"),
      brandColor: "#007bff",
      conversationStarters: [],
      persistentMenus: [],
      authorizedDomains: ["example.com"],
    })
    expect(webchat.status, webchat.text).toBe(201)
    const contact = await api.post<{ id: string }>("/v1/contacts", {
      channel: "webchat",
      inboxId: webchat.json.inboxId,
      firstName: "Hook",
      lastName: "Parity",
      email: "",
      gender: "unknown",
    })
    expect(contact.status, contact.text).toBe(200)

    const flow = await api.post<{ id: string }>("/v1/flows", {
      name: uniqueName("Make flow"),
      folderId: null,
      publish: true,
      nodes: [
        {
          id: "1",
          position: { x: 0, y: 0 },
          measured: { width: 288, height: 100 },
          type: "performAction",
          data: {
            name: "Send to Make",
            isStartNode: true,
            details: {
              steps: [
                {
                  id: "2",
                  stepType: "make",
                  events: ["lead_ready"],
                  states: [
                    { id: "3", stateType: "success" },
                    { id: "4", stateType: "error" },
                  ],
                },
              ],
            },
          },
        },
      ],
      edges: [],
    })
    expect(flow.status, flow.text).toBe(201)

    const since = await fake.cursor()
    const sent = await api.post(`/v1/contacts/${encodeURIComponent(`id:${contact.json.id}`)}/flows`, { flowId: flow.json.id })
    expect(sent.status, sent.text).toBe(204)

    const calls = await eventually(
      async () => {
        const records = await upstreamCalls({ host, since, method: "POST" })
        expect(records.map((record) => record.path).sort()).toEqual(["/make", "/n8n"])
        return records
      },
      { timeoutMs: 30_000, intervalMs: 250, description: "make step delivery" },
    )
    expect((await dohLookups(host, since)).length).toBeGreaterThanOrEqual(1)
    for (const call of calls) {
      expect(call).toMatchObject({ scheme: "https", method: "POST", unmatched: false })
      expect(call.headers["content-type"]).toMatch(/^application\/json/)
      const body = JSON.parse(call.bodyText) as Record<string, unknown>
      expect(keysOf(body)).toEqual(["contact", "contact_sequences", "custom_fields", "event", "tags"])
      expect(body).toMatchObject({
        event: "lead_ready",
        contact: { id: contact.json.id, firstName: "Hook", lastName: "Parity" },
        custom_fields: [],
        tags: [],
        contact_sequences: [],
      })
    }
    expect(await upstreamCalls({ host, since, path: /^\/unused$/ })).toHaveLength(0)
  })
})
