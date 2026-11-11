// Bot message analytics: botMessagesByResult, botMessagesWithResponse,
// botMessagesNoResponse, botMessagesAiProviders.
//
// Notes on the original's behavior that the port must keep:
// - by-result day series are zero-filled per day for `success` and
//   `fallback`; a filled row keeps the responseType/aiProvider of the FIRST
//   SQL group of that (day, result). Several groups per (day, result) come
//   in no defined order, so the seed has one group per (day, result).
// - every by-result row reports `hasResponse: true`.
// - granularity `hour` without a filter reads the hourly cagg (refresh lag,
//   not deterministic), so hour/minute reads here always pass a filter or
//   use `minute` (raw table).

import { beforeAll, describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { type Api, day, expectInvalid, ext, sortBy, syntheticId, tenantApi, uniqueName } from "./helpers.ts"

const UTC_RANGE = { from: "2025-05-01T00:00:00Z", to: "2025-05-03T23:59:59Z" }
const HCM_RANGE = { from: "2025-05-01T00:00:00+07:00", to: "2025-05-03T23:59:59+07:00", timezone: "Asia/Ho_Chi_Minh" }

type Row = { timestamp: string; responseType: string; result?: string; aiProvider: string; count: number; hasResponse: boolean }
const zero = (key: string, result: "success" | "fallback") => ({
  timestamp: day(key),
  hasResponse: true,
  responseType: "none",
  result,
  aiProvider: "none",
  count: 0,
})
const hit = (timestamp: string, result: string, responseType: string, aiProvider: string, count: number) => ({
  timestamp,
  hasResponse: true,
  responseType,
  result,
  aiProvider,
  count,
})
const sortRows = (rows: Row[]) => sortBy(rows, (row) => `${row.timestamp}|${row.responseType}|${row.result ?? ""}|${row.aiProvider}`)

describe("bot message analytics over seeded events", () => {
  let tenant: Tenant
  let api: Api

  beforeAll(async () => {
    ;({ tenant, api } = await tenantApi("an-bot"))
    const ws = tenant.workspaceId
    const fx = await ext()
    const i1 = (await fx.createInbox({ workspaceId: ws, name: uniqueName("I1"), channel: "messenger" })).inboxId
    const i2 = (await fx.createInbox({ workspaceId: ws, name: uniqueName("I2"), channel: "telegram" })).inboxId
    const m = { channel: "messenger", inboxId: i1 }
    const t = { channel: "telegram", inboxId: i2 }
    const event = (occurredAt: string, rest: Record<string, unknown>) => ({
      workspaceId: ws,
      messageId: syntheticId(),
      conversationId: syntheticId(),
      occurredAt,
      hasResponse: true,
      ...rest,
    })
    await fx.insertBotMessageEvents([
      event("2025-05-01T10:00:00Z", { responseType: "ai_agent", routeType: "agent", result: "success", aiProvider: "openai", ...m }),
      event("2025-05-01T11:00:00Z", { responseType: "ai_agent", routeType: "agent", result: "success", aiProvider: "openai", ...m }),
      event("2025-05-02T09:00:00Z", { responseType: "flow", routeType: "flow", result: "success", aiProvider: null, ...t }),
      // UTC 05-02, Ho Chi Minh City 05-03
      event("2025-05-02T20:00:00Z", { responseType: "ai_agent", routeType: "fallback", result: "fallback", aiProvider: "anthropic", ...m }),
      event("2025-05-03T05:00:00Z", { hasResponse: false, responseType: "none", result: null, aiProvider: null, ...m }),
      // an older event without channel / inbox
      event("2025-05-03T06:00:00Z", { hasResponse: false, responseType: "none", result: null, aiProvider: null }),
      event("2025-05-03T07:00:00Z", { responseType: "automated_response", result: "success", aiProvider: null, ...m }),
      // outside the ranges
      event("2025-04-30T12:00:00Z", { responseType: "ai_agent", result: "success", aiProvider: "openai", ...m }),
    ])
  })

  describe("analytics.botMessagesByResult", () => {
    it("zero-fills success and fallback per UTC day (granularity day is the default)", async () => {
      const response = await api.get("/v1/analytics/bot-messages-by-result", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({
        data: [
          hit(day("2025-05-01"), "success", "ai_agent", "openai", 2),
          zero("2025-05-01", "fallback"),
          hit(day("2025-05-02"), "success", "flow", "none", 1),
          hit(day("2025-05-02"), "fallback", "ai_agent", "anthropic", 1),
          hit(day("2025-05-03"), "success", "automated_response", "none", 1),
          zero("2025-05-03", "fallback"),
        ],
      })
    })

    it("buckets by local day of the timezone", async () => {
      const response = await api.get("/v1/analytics/bot-messages-by-result", { query: { ...HCM_RANGE, granularity: "day" } })
      expect(response.json).toEqual({
        data: [
          hit(day("2025-05-01"), "success", "ai_agent", "openai", 2),
          zero("2025-05-01", "fallback"),
          hit(day("2025-05-02"), "success", "flow", "none", 1),
          zero("2025-05-02", "fallback"),
          hit(day("2025-05-03"), "success", "automated_response", "none", 1),
          hit(day("2025-05-03"), "fallback", "ai_agent", "anthropic", 1),
        ],
      })
    })

    it("hour buckets from the raw table under a filter, without zero-fill", async () => {
      const response = await api.get("/v1/analytics/bot-messages-by-result", { query: { ...UTC_RANGE, granularity: "hour", channels: "messenger" } })
      expect(response.json).toEqual({
        data: [
          hit("2025-05-01T10:00:00.000Z", "success", "ai_agent", "openai", 1),
          hit("2025-05-01T11:00:00.000Z", "success", "ai_agent", "openai", 1),
          hit("2025-05-02T20:00:00.000Z", "fallback", "ai_agent", "anthropic", 1),
          hit("2025-05-03T07:00:00.000Z", "success", "automated_response", "none", 1),
        ],
      })
    })

    it("monthly buckets past 60 days sum the counts per result", async () => {
      const response = await api.get<{ data: Row[] }>("/v1/analytics/bot-messages-by-result", {
        query: { from: "2025-03-01T00:00:00Z", to: "2025-05-31T23:59:59Z" },
      })
      // responseType / aiProvider of a month with several groups is the first
      // group's, which has no defined order: only counts are compared.
      expect(response.json.data.map(({ timestamp, result, count, hasResponse }) => ({ timestamp, result, count, hasResponse }))).toEqual([
        { timestamp: day("2025-03-01"), result: "success", count: 0, hasResponse: true },
        { timestamp: day("2025-03-01"), result: "fallback", count: 0, hasResponse: true },
        { timestamp: day("2025-04-01"), result: "success", count: 1, hasResponse: true },
        { timestamp: day("2025-04-01"), result: "fallback", count: 0, hasResponse: true },
        { timestamp: day("2025-05-01"), result: "success", count: 4, hasResponse: true },
        { timestamp: day("2025-05-01"), result: "fallback", count: 1, hasResponse: true },
      ])
      expect(response.json.data[0]).toEqual(zero("2025-03-01", "success"))
      expect(response.json.data[2]).toEqual(hit(day("2025-04-01"), "success", "ai_agent", "openai", 1))
    })

    it("rejects an unknown granularity (422)", async () => {
      expectInvalid(await api.get("/v1/analytics/bot-messages-by-result", { query: { ...UTC_RANGE, granularity: "month" } }), "granularity")
    })
  })

  describe("analytics.botMessagesWithResponse", () => {
    it("lists answered groups per day (order inside a day is not defined)", async () => {
      const response = await api.get<{ data: Row[] }>("/v1/analytics/bot-messages-with-response", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(response.json.data.map((row) => row.timestamp)).toEqual([...response.json.data.map((row) => row.timestamp)].sort())
      expect(sortRows(response.json.data)).toEqual(
        sortRows([
          hit(day("2025-05-01"), "success", "ai_agent", "openai", 2),
          hit(day("2025-05-02"), "success", "flow", "none", 1),
          hit(day("2025-05-02"), "fallback", "ai_agent", "anthropic", 1),
          hit(day("2025-05-03"), "success", "automated_response", "none", 1),
        ]),
      )
    })

    it("buckets by local day and by minute", async () => {
      const local = await api.get<{ data: Row[] }>("/v1/analytics/bot-messages-with-response", { query: HCM_RANGE })
      expect(sortRows(local.json.data)).toEqual(
        sortRows([
          hit(day("2025-05-01"), "success", "ai_agent", "openai", 2),
          hit(day("2025-05-02"), "success", "flow", "none", 1),
          hit(day("2025-05-03"), "fallback", "ai_agent", "anthropic", 1),
          hit(day("2025-05-03"), "success", "automated_response", "none", 1),
        ]),
      )
      const minute = await api.get("/v1/analytics/bot-messages-with-response", {
        query: { from: "2025-05-01T00:00:00Z", to: "2025-05-01T23:59:59Z", granularity: "minute" },
      })
      expect(minute.json).toEqual({
        data: [
          hit("2025-05-01T10:00:00.000Z", "success", "ai_agent", "openai", 1),
          hit("2025-05-01T11:00:00.000Z", "success", "ai_agent", "openai", 1),
        ],
      })
    })
  })

  describe("analytics.botMessagesNoResponse", () => {
    it("counts unanswered messages per day", async () => {
      const response = await api.get("/v1/analytics/bot-messages-no-response", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({
        data: [{ timestamp: day("2025-05-03"), hasResponse: false, responseType: "none", aiProvider: "none", count: 2 }],
      })
    })

    it("an hour read under a channel filter skips events without a channel", async () => {
      const response = await api.get("/v1/analytics/bot-messages-no-response", { query: { ...UTC_RANGE, granularity: "hour", channels: "messenger" } })
      expect(response.json).toEqual({
        data: [{ timestamp: "2025-05-03T05:00:00.000Z", hasResponse: false, responseType: "none", aiProvider: "none", count: 1 }],
      })
    })

    it("minute buckets read the raw table", async () => {
      const response = await api.get("/v1/analytics/bot-messages-no-response", { query: { ...UTC_RANGE, granularity: "minute" } })
      expect(response.json).toEqual({
        data: [
          { timestamp: "2025-05-03T05:00:00.000Z", hasResponse: false, responseType: "none", aiProvider: "none", count: 1 },
          { timestamp: "2025-05-03T06:00:00.000Z", hasResponse: false, responseType: "none", aiProvider: "none", count: 1 },
        ],
      })
    })
  })

  describe("analytics.botMessagesAiProviders", () => {
    it("shares the AI agent answers per provider", async () => {
      const response = await api.get("/v1/analytics/bot-messages-ai-providers", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({
        data: [
          { aiProvider: "openai", count: 2, percentage: (2 / 3) * 100 },
          { aiProvider: "anthropic", count: 1, percentage: (1 / 3) * 100 },
        ],
      })
      expect((await api.get("/v1/analytics/bot-messages-ai-providers", { query: { ...UTC_RANGE, channels: "telegram" } })).json).toEqual({
        data: [],
      })
    })

    it("rejects a missing range (422)", async () => {
      expectInvalid(await api.get("/v1/analytics/bot-messages-ai-providers", { query: { from: UTC_RANGE.from } }), "to")
    })
  })
})
