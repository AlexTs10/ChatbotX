// Human agent and message analytics: messagesByAdmin, messagesBySender,
// humanAgentStats, agentResponseSeries. Per-admin rows are built from the
// CURRENT workspace members; their order is not documented, so the tests
// sort them by adminId.

import { beforeAll, describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { type Api, addAgent, day, expectInvalid, ext, sortBy, syntheticId, tenantApi, uniqueName } from "./helpers.ts"

const UTC_RANGE = { from: "2025-04-01T00:00:00Z", to: "2025-04-03T23:59:59Z" }
const HCM_RANGE = { from: "2025-04-01T00:00:00+07:00", to: "2025-04-03T23:59:59+07:00", timezone: "Asia/Ho_Chi_Minh" }

type Member = { userId: string; name: string; email: string }
const byAdmin = <T extends { adminId: string }>(rows: T[]) => sortBy(rows, (row) => row.adminId.padStart(20, "0"))

const NO_SLA = {
  responseSlaEligibleCount: 0,
  responseSlaMetCount: 0,
  responseSlaMetPercent: null,
  firstResponseSlaEligibleCount: 0,
  firstResponseSlaMetCount: 0,
  firstResponseSlaMetPercent: null,
}
const NO_RESPONSES = {
  avgResponseSeconds: null,
  avgWorkingResponseSeconds: null,
  avgFirstResponseSeconds: null,
  avgWorkingFirstResponseSeconds: null,
  responseCount: 0,
  firstResponseCount: 0,
}

const seedResponses = async (ws: string, u0: string, u1: string, inboxes: { i1: string; i2: string }) => {
  const at = (iso: string, seconds: number) => new Date(Date.parse(iso) - seconds * 1000).toISOString()
  const row = (adminId: string, occurredAt: string, responseSeconds: number, rest: Record<string, unknown>) => ({
    workspaceId: ws,
    conversationId: syntheticId(),
    contactId: syntheticId(),
    adminId,
    occurredAt,
    awaitingSince: at(occurredAt, responseSeconds),
    measuredFrom: at(occurredAt, responseSeconds),
    responseSeconds,
    isFirstResponse: false,
    channel: "messenger",
    inboxId: inboxes.i1,
    ...rest,
  })
  await (await ext()).insertAgentResponseEvents([
    row(u0, "2025-04-01T10:00:00Z", 100, { workingResponseSeconds: 60, isFirstResponse: true, slaTargetSeconds: 300, slaMet: true }),
    row(u0, "2025-04-01T11:00:00Z", 301, { workingResponseSeconds: null, slaTargetSeconds: 300, slaMet: false }),
    row(u1, "2025-04-02T09:00:00Z", 50, { workingResponseSeconds: 50, isFirstResponse: true, channel: "telegram", inboxId: inboxes.i2 }),
    // 04-02 in UTC, 04-03 in Ho Chi Minh City
    row(u0, "2025-04-02T20:00:00Z", 30, { workingResponseSeconds: 30, slaTargetSeconds: 300, slaMet: true }),
    // an admin who is no longer a member: dropped from every per-admin read
    row(syntheticId(), "2025-04-01T12:00:00Z", 10, {}),
  ])
}

describe("message and agent analytics (no SLA policy)", () => {
  let tenant: Tenant
  let api: Api
  let owner: Member
  let agent: Member
  let idle: Member
  let i1: string
  let i2: string

  beforeAll(async () => {
    ;({ tenant, api } = await tenantApi("an-agents"))
    const ws = tenant.workspaceId
    owner = { userId: tenant.userId, name: "Parity an-agents", email: tenant.email }
    agent = { ...(await addAgent(ws, "Agent One")) }
    idle = { ...(await addAgent(ws, "Agent Idle")) }
    const fx = await ext()
    i1 = (await fx.createInbox({ workspaceId: ws, name: uniqueName("I1"), channel: "messenger" })).inboxId
    i2 = (await fx.createInbox({ workspaceId: ws, name: uniqueName("I2"), channel: "telegram" })).inboxId
    const [k1, k2, k3] = [syntheticId(), syntheticId(), syntheticId()] as [string, string, string]
    const m = { channel: "messenger", inboxId: i1 }
    const t = { channel: "telegram", inboxId: i2 }
    await fx.insertMessageEvents([
      { workspaceId: ws, contactId: k1, eventType: "message_human_sent", occurredAt: "2025-04-01T10:00:00Z", senderType: "human", adminId: owner.userId, ...m },
      { workspaceId: ws, contactId: k2, eventType: "message_human_sent", occurredAt: "2025-04-01T11:00:00Z", senderType: "human", adminId: owner.userId, ...m },
      { workspaceId: ws, contactId: k1, eventType: "message_human_sent", occurredAt: "2025-04-02T09:00:00Z", senderType: "human", adminId: agent.userId, ...t },
      { workspaceId: ws, contactId: k1, eventType: "message_bot_sent", occurredAt: "2025-04-01T12:00:00Z", senderType: "bot", ...m },
      { workspaceId: ws, contactId: k3, eventType: "message_bot_sent", occurredAt: "2025-04-03T23:30:00Z", senderType: "bot", ...t },
      // outside the UTC range, inside the Ho Chi Minh City range (04-01 06:00 local)
      { workspaceId: ws, contactId: k1, eventType: "message_human_sent", occurredAt: "2025-03-31T23:00:00Z", senderType: "human", adminId: owner.userId, ...m },
      // no sender type / no channel: never in the by-sender series
      { workspaceId: ws, contactId: k2, eventType: "message_bot_sent", occurredAt: "2025-04-02T00:00:00Z" },
    ])
    const [v1, v2] = [syntheticId(), syntheticId()] as [string, string]
    await fx.insertConversationEvents([
      { workspaceId: ws, conversationId: v1, eventType: "conversation_assigned", occurredAt: "2025-04-01T10:00:00Z", toAssignee: owner.userId, ...m },
      { workspaceId: ws, conversationId: v2, eventType: "conversation_assigned", occurredAt: "2025-04-02T10:00:00Z", toAssignee: owner.userId, ...m },
      { workspaceId: ws, conversationId: v1, eventType: "conversation_assigned", occurredAt: "2025-04-03T10:00:00Z", toAssignee: agent.userId, ...t },
    ])
    await seedResponses(ws, owner.userId, agent.userId, { i1, i2 })
  })

  describe("analytics.messagesByAdmin", () => {
    it("counts human messages per current member", async () => {
      const response = await api.get<{ data: Array<{ adminId: string }> }>("/v1/analytics/messages-by-admin", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(byAdmin(response.json.data)).toEqual(
        byAdmin([
          { adminId: owner.userId, count: 2, userName: owner.name, userEmail: owner.email },
          { adminId: agent.userId, count: 1, userName: agent.name, userEmail: agent.email },
          { adminId: idle.userId, count: 0, userName: idle.name, userEmail: idle.email },
        ]),
      )
    })

    it("narrows to a channel", async () => {
      const response = await api.get<{ data: Array<{ adminId: string; count: number }> }>("/v1/analytics/messages-by-admin", {
        query: { ...UTC_RANGE, channels: "telegram" },
      })
      expect(byAdmin(response.json.data).map((row) => [row.adminId, row.count])).toEqual(
        byAdmin([
          { adminId: owner.userId, count: 0 },
          { adminId: agent.userId, count: 1 },
          { adminId: idle.userId, count: 0 },
        ]).map((row) => [row.adminId, row.count]),
      )
    })
  })

  describe("analytics.messagesBySender", () => {
    it("counts messages per day, channel and sender type", async () => {
      const response = await api.get("/v1/analytics/messages-by-sender", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({
        data: [
          { timestamp: day("2025-04-01"), channel: "messenger", senderType: "bot", count: 1 },
          { timestamp: day("2025-04-01"), channel: "messenger", senderType: "human", count: 2 },
          { timestamp: day("2025-04-02"), channel: "telegram", senderType: "human", count: 1 },
          { timestamp: day("2025-04-03"), channel: "telegram", senderType: "bot", count: 1 },
        ],
      })
    })

    it("buckets by local day of the timezone", async () => {
      const response = await api.get("/v1/analytics/messages-by-sender", { query: HCM_RANGE })
      expect(response.json).toEqual({
        data: [
          { timestamp: day("2025-04-01"), channel: "messenger", senderType: "bot", count: 1 },
          { timestamp: day("2025-04-01"), channel: "messenger", senderType: "human", count: 3 },
          { timestamp: day("2025-04-02"), channel: "telegram", senderType: "human", count: 1 },
        ],
      })
    })

    it("uses monthly buckets with granularity month", async () => {
      const response = await api.get("/v1/analytics/messages-by-sender", { query: { ...UTC_RANGE, granularity: "month" } })
      expect(response.json).toEqual({
        data: [
          { timestamp: day("2025-04-01"), channel: "messenger", senderType: "bot", count: 1 },
          { timestamp: day("2025-04-01"), channel: "messenger", senderType: "human", count: 2 },
          { timestamp: day("2025-04-01"), channel: "telegram", senderType: "bot", count: 1 },
          { timestamp: day("2025-04-01"), channel: "telegram", senderType: "human", count: 1 },
        ],
      })
    })

    it("rejects an unknown granularity (422)", async () => {
      expectInvalid(await api.get("/v1/analytics/messages-by-sender", { query: { ...UTC_RANGE, granularity: "hour" } }), "granularity")
    })
  })

  describe("analytics.humanAgentStats", () => {
    it("merges messages, assignments and response times per member", async () => {
      const response = await api.get<{ data: Array<{ adminId: string }> }>("/v1/analytics/human-agent-stats", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(byAdmin(response.json.data)).toEqual(
        byAdmin([
          {
            adminId: owner.userId,
            userName: owner.name,
            userEmail: owner.email,
            assignedConversations: 2,
            messagesSent: 2,
            uniqueContacts: 2,
            avgResponseSeconds: 144,
            avgWorkingResponseSeconds: 45,
            avgFirstResponseSeconds: 100,
            avgWorkingFirstResponseSeconds: 60,
            responseCount: 3,
            firstResponseCount: 1,
            ...NO_SLA,
          },
          {
            adminId: agent.userId,
            userName: agent.name,
            userEmail: agent.email,
            assignedConversations: 1,
            messagesSent: 1,
            uniqueContacts: 1,
            avgResponseSeconds: 50,
            avgWorkingResponseSeconds: 50,
            avgFirstResponseSeconds: 50,
            avgWorkingFirstResponseSeconds: 50,
            responseCount: 1,
            firstResponseCount: 1,
            ...NO_SLA,
          },
          {
            adminId: idle.userId,
            userName: idle.name,
            userEmail: idle.email,
            assignedConversations: 0,
            messagesSent: 0,
            uniqueContacts: 0,
            ...NO_RESPONSES,
            ...NO_SLA,
          },
        ]),
      )
    })

    it("narrows every measure to a channel", async () => {
      const response = await api.get<{ data: Array<{ adminId: string }> }>("/v1/analytics/human-agent-stats", {
        query: { ...UTC_RANGE, channels: "telegram" },
      })
      expect(byAdmin(response.json.data)).toEqual(
        byAdmin([
          { adminId: owner.userId, userName: owner.name, userEmail: owner.email, assignedConversations: 0, messagesSent: 0, uniqueContacts: 0, ...NO_RESPONSES, ...NO_SLA },
          {
            adminId: agent.userId,
            userName: agent.name,
            userEmail: agent.email,
            assignedConversations: 1,
            messagesSent: 1,
            uniqueContacts: 1,
            avgResponseSeconds: 50,
            avgWorkingResponseSeconds: 50,
            avgFirstResponseSeconds: 50,
            avgWorkingFirstResponseSeconds: 50,
            responseCount: 1,
            firstResponseCount: 1,
            ...NO_SLA,
          },
          { adminId: idle.userId, userName: idle.name, userEmail: idle.email, assignedConversations: 0, messagesSent: 0, uniqueContacts: 0, ...NO_RESPONSES, ...NO_SLA },
        ]),
      )
    })
  })
})

describe("agent response analytics with an SLA policy", () => {
  let api: Api
  let owner: Member
  let agent: Member

  beforeAll(async () => {
    let tenant: Tenant
    ;({ tenant, api } = await tenantApi("an-sla"))
    const ws = tenant.workspaceId
    owner = { userId: tenant.userId, name: "Parity an-sla", email: tenant.email }
    agent = { ...(await addAgent(ws, "Agent Sla")) }
    const fx = await ext()
    const i1 = (await fx.createInbox({ workspaceId: ws, name: uniqueName("I1"), channel: "messenger" })).inboxId
    const i2 = (await fx.createInbox({ workspaceId: ws, name: uniqueName("I2"), channel: "telegram" })).inboxId
    await fx.setResponseSlaPolicy({ workspaceId: ws, enabled: true, firstResponseTargetSeconds: 300, nextResponseTargetSeconds: 300 })
    await seedResponses(ws, owner.userId, agent.userId, { i1, i2 })
  })

  const ownerDay1 = () => ({
    adminId: owner.userId,
    date: "2025-04-01",
    userName: owner.name,
    userEmail: owner.email,
    avgResponseSeconds: 201,
    avgWorkingResponseSeconds: 60,
    avgFirstResponseSeconds: 100,
    avgWorkingFirstResponseSeconds: 60,
    responseCount: 2,
    firstResponseCount: 1,
    responseSlaEligibleCount: 2,
    responseSlaMetCount: 1,
    responseSlaMetPercent: 50,
    firstResponseSlaEligibleCount: 1,
    firstResponseSlaMetCount: 1,
    firstResponseSlaMetPercent: 100,
  })
  const ownerLate = (date: string) => ({
    adminId: owner.userId,
    date,
    userName: owner.name,
    userEmail: owner.email,
    avgResponseSeconds: 30,
    avgWorkingResponseSeconds: 30,
    avgFirstResponseSeconds: null,
    avgWorkingFirstResponseSeconds: null,
    responseCount: 1,
    firstResponseCount: 0,
    responseSlaEligibleCount: 1,
    responseSlaMetCount: 1,
    responseSlaMetPercent: 100,
    firstResponseSlaEligibleCount: 0,
    firstResponseSlaMetCount: 0,
    firstResponseSlaMetPercent: null,
  })
  const agentDay = () => ({
    adminId: agent.userId,
    date: "2025-04-02",
    userName: agent.name,
    userEmail: agent.email,
    avgResponseSeconds: 50,
    avgWorkingResponseSeconds: 50,
    avgFirstResponseSeconds: 50,
    avgWorkingFirstResponseSeconds: 50,
    responseCount: 1,
    firstResponseCount: 1,
    ...NO_SLA,
  })

  describe("analytics.agentResponseSeries", () => {
    it("returns per-admin, per-day averages ordered by day then admin", async () => {
      const response = await api.get("/v1/analytics/agent-response-series", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({ data: [ownerDay1(), ownerLate("2025-04-02"), agentDay()] })
    })

    it("buckets by local day of the timezone", async () => {
      const response = await api.get("/v1/analytics/agent-response-series", { query: HCM_RANGE })
      expect(response.json).toEqual({ data: [ownerDay1(), agentDay(), ownerLate("2025-04-03")] })
    })

    it("narrows to a channel", async () => {
      const response = await api.get("/v1/analytics/agent-response-series", { query: { ...UTC_RANGE, channels: "telegram" } })
      expect(response.json).toEqual({ data: [agentDay()] })
    })

    it("rejects a missing range (422)", async () => {
      expectInvalid(await api.get("/v1/analytics/agent-response-series", { query: { to: UTC_RANGE.to } }), "from")
    })
  })

  it("analytics.humanAgentStats reports the SLA outcome while the policy is on", async () => {
    const response = await api.get<{ data: Array<{ adminId: string }> }>("/v1/analytics/human-agent-stats", { query: UTC_RANGE })
    expect(byAdmin(response.json.data)).toEqual(
      byAdmin([
        {
          adminId: owner.userId,
          userName: owner.name,
          userEmail: owner.email,
          assignedConversations: 0,
          messagesSent: 0,
          uniqueContacts: 0,
          avgResponseSeconds: 144,
          avgWorkingResponseSeconds: 45,
          avgFirstResponseSeconds: 100,
          avgWorkingFirstResponseSeconds: 60,
          responseCount: 3,
          firstResponseCount: 1,
          responseSlaEligibleCount: 3,
          responseSlaMetCount: 2,
          responseSlaMetPercent: 66.7,
          firstResponseSlaEligibleCount: 1,
          firstResponseSlaMetCount: 1,
          firstResponseSlaMetPercent: 100,
        },
        { ...agentDay(), date: undefined, assignedConversations: 0, messagesSent: 0, uniqueContacts: 0 },
      ]).map((row) => Object.fromEntries(Object.entries(row).filter(([, value]) => value !== undefined))),
    )
  })
})
