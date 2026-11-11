// Conversation event analytics: conversationHandoffs, conversationFollowUps,
// conversationArchived, conversationAssigned, conversationAssignedByAdmin,
// uniqueConversationsByAdmin. Day series are not zero-filled; a bucket's
// `timestamp` is the local day of `timezone` at 00:00 UTC. The New York range
// crosses the DST change of 2025-03-09.

import { beforeAll, describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { type Api, addAgent, day, expectInvalid, ext, sortBy, syntheticId, tenantApi, uniqueName } from "./helpers.ts"

const UTC_RANGE = { from: "2025-03-08T00:00:00Z", to: "2025-03-11T23:59:59Z" }
const NY_RANGE = { from: "2025-03-08T00:00:00-05:00", to: "2025-03-11T23:59:59-04:00", timezone: "America/New_York" }

describe("conversation analytics over seeded events", () => {
  let tenant: Tenant
  let api: Api
  let owner: { userId: string; name: string; email: string }
  let agent: { userId: string; name: string; email: string }

  beforeAll(async () => {
    ;({ tenant, api } = await tenantApi("an-conv"))
    const ws = tenant.workspaceId
    owner = { userId: tenant.userId, name: "Parity an-conv", email: tenant.email }
    agent = await addAgent(ws, "Agent Conv")
    const fx = await ext()
    const i1 = (await fx.createInbox({ workspaceId: ws, name: uniqueName("I1"), channel: "messenger" })).inboxId
    const i2 = (await fx.createInbox({ workspaceId: ws, name: uniqueName("I2"), channel: "telegram" })).inboxId
    const [v1, v2] = [syntheticId(), syntheticId()] as [string, string]
    const m = { workspaceId: ws, channel: "messenger", inboxId: i1 }
    const t = { workspaceId: ws, channel: "telegram", inboxId: i2 }
    await fx.insertConversationEvents([
      // New York: 03-08 10:00 EST
      { ...m, conversationId: v1, eventType: "conversation_transferred_to_human", occurredAt: "2025-03-08T15:00:00Z" },
      // New York: 03-08 23:30 EST; UTC 03-09
      { ...m, conversationId: v2, eventType: "conversation_transferred_to_human", occurredAt: "2025-03-09T04:30:00Z" },
      // New York: 03-09 12:00 EDT
      { ...t, conversationId: v1, eventType: "conversation_transferred_to_bot", occurredAt: "2025-03-09T16:00:00Z" },
      // New York: 03-09 23:30 EDT; UTC 03-10
      { ...m, conversationId: v1, eventType: "conversation_followed", occurredAt: "2025-03-10T03:30:00Z" },
      { ...m, conversationId: v2, eventType: "conversation_followed", occurredAt: "2025-03-10T12:00:00Z" },
      { ...m, conversationId: v2, eventType: "conversation_unfollowed", occurredAt: "2025-03-10T13:00:00Z" },
      // New York: 03-10 22:00 EDT; UTC 03-11
      { ...m, conversationId: v1, eventType: "conversation_archived", occurredAt: "2025-03-11T02:00:00Z" },
      { ...m, conversationId: v1, eventType: "conversation_unarchived", occurredAt: "2025-03-11T03:00:00Z" },
      { ...m, conversationId: v1, eventType: "conversation_assigned", occurredAt: "2025-03-08T15:00:00Z", toAssignee: owner.userId },
      { ...m, conversationId: v2, eventType: "conversation_assigned", occurredAt: "2025-03-09T16:00:00Z", toAssignee: owner.userId },
      { ...t, conversationId: v1, eventType: "conversation_assigned", occurredAt: "2025-03-10T12:00:00Z", fromAssignee: owner.userId, toAssignee: agent.userId },
      // the same conversation assigned to the owner again
      { ...m, conversationId: v1, eventType: "conversation_assigned", occurredAt: "2025-03-10T13:00:00Z", fromAssignee: agent.userId, toAssignee: owner.userId },
      { ...m, conversationId: v2, eventType: "conversation_unassigned", occurredAt: "2025-03-10T14:00:00Z", fromAssignee: owner.userId },
      // outside both ranges
      { ...m, conversationId: v2, eventType: "conversation_assigned", occurredAt: "2025-03-01T00:00:00Z", toAssignee: agent.userId },
    ])
  })

  describe("analytics.conversationHandoffs", () => {
    it("counts transfers per day and direction", async () => {
      const response = await api.get("/v1/analytics/conversation-handoffs", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({
        data: [
          { timestamp: day("2025-03-08"), direction: "to_human", count: 1 },
          { timestamp: day("2025-03-09"), direction: "to_bot", count: 1 },
          { timestamp: day("2025-03-09"), direction: "to_human", count: 1 },
        ],
      })
    })

    it("buckets by local day across the DST change", async () => {
      const response = await api.get("/v1/analytics/conversation-handoffs", { query: NY_RANGE })
      expect(response.json).toEqual({
        data: [
          { timestamp: day("2025-03-08"), direction: "to_human", count: 2 },
          { timestamp: day("2025-03-09"), direction: "to_bot", count: 1 },
        ],
      })
    })

    it("narrows to a channel", async () => {
      const response = await api.get("/v1/analytics/conversation-handoffs", { query: { ...UTC_RANGE, channels: "telegram" } })
      expect(response.json).toEqual({ data: [{ timestamp: day("2025-03-09"), direction: "to_bot", count: 1 }] })
    })
  })

  describe("analytics.conversationFollowUps", () => {
    it("counts follow events per day", async () => {
      expect((await api.get("/v1/analytics/conversation-followups", { query: UTC_RANGE })).json).toEqual({
        data: [{ timestamp: day("2025-03-10"), count: 2 }],
      })
      expect((await api.get("/v1/analytics/conversation-followups", { query: NY_RANGE })).json).toEqual({
        data: [
          { timestamp: day("2025-03-09"), count: 1 },
          { timestamp: day("2025-03-10"), count: 1 },
        ],
      })
    })
  })

  describe("analytics.conversationArchived", () => {
    it("counts archive events per day", async () => {
      expect((await api.get("/v1/analytics/conversation-archived", { query: UTC_RANGE })).json).toEqual({
        data: [{ timestamp: day("2025-03-11"), count: 1 }],
      })
      expect((await api.get("/v1/analytics/conversation-archived", { query: NY_RANGE })).json).toEqual({
        data: [{ timestamp: day("2025-03-10"), count: 1 }],
      })
      expect((await api.get("/v1/analytics/conversation-archived", { query: { ...UTC_RANGE, channels: "telegram" } })).json).toEqual({
        data: [],
      })
    })
  })

  describe("analytics.conversationAssigned", () => {
    it("counts assignments per day", async () => {
      const expected = {
        data: [
          { timestamp: day("2025-03-08"), count: 1 },
          { timestamp: day("2025-03-09"), count: 1 },
          { timestamp: day("2025-03-10"), count: 2 },
        ],
      }
      expect((await api.get("/v1/analytics/conversation-assigned", { query: UTC_RANGE })).json).toEqual(expected)
      expect((await api.get("/v1/analytics/conversation-assigned", { query: NY_RANGE })).json).toEqual(expected)
    })
  })

  describe("analytics.conversationAssignedByAdmin", () => {
    it("counts assignments per current member", async () => {
      const response = await api.get<{ data: Array<{ toAssignee: string }> }>("/v1/analytics/conversation-assigned-by-admin", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(sortBy(response.json.data, (row) => row.toAssignee.padStart(20, "0"))).toEqual(
        sortBy(
          [
            { toAssignee: owner.userId, count: 3, userName: owner.name, userEmail: owner.email },
            { toAssignee: agent.userId, count: 1, userName: agent.name, userEmail: agent.email },
          ],
          (row) => row.toAssignee.padStart(20, "0"),
        ),
      )
    })
  })

  describe("analytics.uniqueConversationsByAdmin", () => {
    it("counts distinct assigned conversations per current member", async () => {
      const response = await api.get<{ data: Array<{ toAssignee: string }> }>("/v1/analytics/unique-conversations-by-admin", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(sortBy(response.json.data, (row) => row.toAssignee.padStart(20, "0"))).toEqual(
        sortBy(
          [
            { toAssignee: owner.userId, count: 2, userName: owner.name, userEmail: owner.email },
            { toAssignee: agent.userId, count: 1, userName: agent.name, userEmail: agent.email },
          ],
          (row) => row.toAssignee.padStart(20, "0"),
        ),
      )
      const telegram = await api.get<{ data: Array<{ toAssignee: string; count: number }> }>(
        "/v1/analytics/unique-conversations-by-admin",
        { query: { ...UTC_RANGE, channels: "telegram" } },
      )
      expect(Object.fromEntries(telegram.json.data.map((row) => [row.toAssignee, row.count]))).toEqual({
        [owner.userId]: 0,
        [agent.userId]: 1,
      })
    })
  })

  it.each([
    "conversation-handoffs",
    "conversation-followups",
    "conversation-archived",
    "conversation-assigned",
    "conversation-assigned-by-admin",
    "unique-conversations-by-admin",
  ])("%s rejects a missing range and an unknown channel (422)", async (route) => {
    expectInvalid(await api.get(`/v1/analytics/${route}`, { query: { from: UTC_RANGE.from } }), "to")
    expectInvalid(await api.get(`/v1/analytics/${route}`, { query: { ...UTC_RANGE, channels: "fax" } }), "channels.0")
  })
})
