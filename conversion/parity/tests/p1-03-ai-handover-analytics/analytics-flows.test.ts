// analytics.flowStats, analytics.resetFlowStats, analytics.flowSmartDelayStats.
//
// Flows are created through the public API (flows.create with a spec and
// publish: true), which also writes the flow's FlowAnalyticsSession. Node
// stats rows are seeded against that session.

import { describe, expect, it } from "vitest"
import type { CreatedContact } from "../../src/fixtures/ext/p1-03.ts"
import { type Api, expectError, ext, tenantApi, uniqueName } from "./helpers.ts"

type FlowNode = { id: string; type: string; data: { details: { steps?: Array<{ id: string; buttons?: Array<{ id: string }> }>; quickReplies?: Array<{ id: string }> } } }
type FlowVersion = { id: string; isDraft: boolean; nodes: FlowNode[] }
type Flow = { id: string; currentVersionId: string | null; draftVersionId: string | null; flowVersions: FlowVersion[] }

const SPEC = {
  formatVersion: 1,
  name: "Stats flow",
  steps: [
    { type: "send", text: "Hello, pick one", buttons: [{ text: "Yes" }, { text: "No" }] },
    { type: "wait", duration: 1, unit: "days" },
    { type: "send", text: "Bye" },
  ],
}

const createFlow = async (api: Api, publish: boolean) => {
  const created = await api.post<{ id: string }>("/v1/flows", { name: uniqueName("Stats"), folderId: null, spec: SPEC, publish })
  expect(created.status).toBe(201)
  const flow = await api.get<Flow>(`/v1/flows/${created.json.id}`)
  expect(flow.status).toBe(200)
  return flow.json
}

const versionNodes = (flow: Flow, which: "published" | "draft") => {
  const version = flow.flowVersions.find((v) => (which === "draft" ? v.isDraft : v.id === flow.currentVersionId))
  if (!version) throw new Error(`no ${which} version`)
  return { versionId: version.id, nodes: version.nodes }
}

const buttonIdsOf = (node: FlowNode) => [
  ...(node.data.details.quickReplies ?? []).map((b) => b.id),
  ...(node.data.details.steps ?? []).flatMap((step) => (step.buttons ?? []).map((b) => b.id)),
]

const zeroNode = { "message:sent": 0, "message:seen": 0, "message:delivered": 0, "flow:clicked": { clicked: 0, totalUsers: 0 }, "message:failed": 0 }

describe("analytics.flowStats / analytics.resetFlowStats", () => {
  it("aggregates node stats of the live session; reset starts a new empty session", async () => {
    const { tenant, api } = await tenantApi("an-flow-stats")
    const ws = tenant.workspaceId
    const flow = await createFlow(api, true)
    const sendNodes = versionNodes(flow, "published").nodes.filter((n) => n.type === "sendMessage")
    expect(sendNodes).toHaveLength(2)
    const withButtons = sendNodes.find((n) => buttonIdsOf(n).length > 0) as FlowNode
    const plain = sendNodes.find((n) => n !== withButtons) as FlowNode
    const [yes, no] = buttonIdsOf(withButtons) as [string, string]
    expect(buttonIdsOf(withButtons)).toHaveLength(2)

    const fx = await ext()
    const analyticsId = await fx.findFlowAnalyticsSessionId({ workspaceId: ws, flowId: flow.id })
    expect(analyticsId).not.toBeNull()
    const { inboxId } = await fx.createInbox({ workspaceId: ws, name: uniqueName("Inbox"), channel: "messenger" })
    const contact = (contactLastReadAt: string | null) =>
      fx.createContact({ workspaceId: ws, inboxId, channel: "messenger", contactLastReadAt })
    // K1 read after the delivery, K2 never read, K3 read before it, K4 failed.
    const k1 = await contact("2025-06-02T00:00:00Z")
    const k2 = await contact(null)
    const k3 = await contact("2025-05-31T00:00:00Z")
    const k4 = await contact(null)
    const stat = (k: CreatedContact, nodeId: string, eventType: "message:delivered" | "message:failed" | "flow:clicked", occurredAt: string, buttonId?: string) => ({
      workspaceId: ws,
      flowId: flow.id,
      analyticsId: analyticsId as string,
      nodeId,
      buttonId: buttonId ?? null,
      contactId: k.contactId,
      contactInboxId: k.contactInboxId,
      eventType,
      occurredAt,
    })
    await fx.insertFlowNodeStats([
      stat(k1, withButtons.id, "message:delivered", "2025-06-01T10:00:00Z"),
      stat(k2, withButtons.id, "message:delivered", "2025-06-01T10:00:00Z"),
      stat(k3, withButtons.id, "message:delivered", "2025-06-01T10:00:00Z"),
      { ...stat(k4, withButtons.id, "message:failed", "2025-06-01T10:00:00Z"), errorContent: "blocked" },
      stat(k1, withButtons.id, "flow:clicked", "2025-06-01T10:01:00Z", yes),
      stat(k1, withButtons.id, "flow:clicked", "2025-06-01T10:02:00Z", yes),
      stat(k2, withButtons.id, "flow:clicked", "2025-06-01T10:03:00Z", yes),
      stat(k3, withButtons.id, "flow:clicked", "2025-06-01T10:04:00Z", no),
      stat(k1, plain.id, "message:delivered", "2025-06-03T00:00:00Z"),
    ])

    const response = await api.get(`/v1/analytics/flows/${flow.id}/stats`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual({
      [withButtons.id]: {
        node: { "message:sent": 4, "message:seen": 1, "message:delivered": 3, "flow:clicked": { clicked: 4, totalUsers: 3 }, "message:failed": 1 },
        buttons: { [yes]: { buttonId: yes, clicks: 2 }, [no]: { buttonId: no, clicks: 1 } },
      },
      [plain.id]: {
        node: { "message:sent": 1, "message:seen": 0, "message:delivered": 1, "flow:clicked": { clicked: 0, totalUsers: 1 }, "message:failed": 0 },
        buttons: {},
      },
    })

    const reset = await api.delete(`/v1/analytics/flows/${flow.id}/stats`)
    expect(reset.status).toBe(204)
    expect(reset.text).toBe("")
    const fresh = await fx.findFlowAnalyticsSessionId({ workspaceId: ws, flowId: flow.id })
    expect(fresh).not.toBeNull()
    expect(fresh).not.toBe(analyticsId)
    // The cached stats were invalidated: the new session has no rows.
    expect((await api.get(`/v1/analytics/flows/${flow.id}/stats`)).json).toEqual({
      [withButtons.id]: { node: zeroNode, buttons: { [yes]: { buttonId: yes, clicks: 0 }, [no]: { buttonId: no, clicks: 0 } } },
      [plain.id]: { node: zeroNode, buttons: {} },
    })
  })

  it("is {} for a draft-only flow, an unknown flow and another workspace's flow", async () => {
    const owner = await tenantApi("an-flow-empty")
    const other = await tenantApi("an-flow-empty-other")
    const draft = await createFlow(owner.api, false)
    expect(draft.currentVersionId).toBeNull()
    expect((await owner.api.get(`/v1/analytics/flows/${draft.id}/stats`)).json).toEqual({})
    expect((await owner.api.get("/v1/analytics/flows/1/stats")).json).toEqual({})
    const published = await createFlow(owner.api, true)
    expect((await other.api.get(`/v1/analytics/flows/${published.id}/stats`)).json).toEqual({})
  })

  it("reset of an unknown or foreign flow is a 204 no-op", async () => {
    const owner = await tenantApi("an-flow-reset")
    const other = await tenantApi("an-flow-reset-other")
    const flow = await createFlow(owner.api, true)
    const fx = await ext()
    const before = await fx.findFlowAnalyticsSessionId({ workspaceId: owner.tenant.workspaceId, flowId: flow.id })
    expect((await other.api.delete(`/v1/analytics/flows/${flow.id}/stats`)).status).toBe(204)
    expect((await owner.api.delete("/v1/analytics/flows/1/stats")).status).toBe(204)
    expect(await fx.findFlowAnalyticsSessionId({ workspaceId: owner.tenant.workspaceId, flowId: flow.id })).toBe(before)
  })
})

describe("analytics.flowSmartDelayStats", () => {
  it("counts waiting and sent contacts per wait node of the draft", async () => {
    const { tenant, api } = await tenantApi("an-smart-delay")
    const ws = tenant.workspaceId
    const flow = await createFlow(api, true)
    const { versionId, nodes } = versionNodes(flow, "draft")
    const waitNodes = nodes.filter((n) => n.type === "wait")
    expect(waitNodes).toHaveLength(1)
    const wait = waitNodes[0] as FlowNode
    const stepId = wait.data.details.steps?.[0]?.id as string
    expect(stepId).toEqual(expect.any(String))

    const fx = await ext()
    const { inboxId } = await fx.createInbox({ workspaceId: ws, name: uniqueName("Inbox"), channel: "messenger" })
    const k = await fx.createContact({ workspaceId: ws, inboxId, channel: "messenger" })
    const future = "2099-01-01T00:00:00Z"
    const row = (status: "pending" | "scheduled" | "completed" | "failed" | "canceled", step: string | null = stepId) => ({
      workspaceId: ws,
      flowId: flow.id,
      flowVersionId: versionId,
      contactInboxId: k.contactInboxId,
      conversationId: k.conversationId,
      nodeId: wait.id,
      stepId: step,
      type: "waitNode" as const,
      status,
      triggerAt: future,
    })
    await fx.insertSmartDelays([
      row("pending"),
      row("pending"),
      row("scheduled"),
      row("completed"),
      row("completed"),
      row("completed"),
      row("canceled"),
      row("failed"),
      row("pending", "not-a-step-of-this-flow"),
      row("pending", null),
    ])
    const response = await api.get(`/v1/analytics/flows/${flow.id}/smart-delay-stats`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ [wait.id]: { waiting: 3, sent: 3 } })
  })

  it("lists wait nodes with zeros when nothing waits, and {} for a flow without wait nodes", async () => {
    const { api } = await tenantApi("an-smart-delay-empty")
    const flow = await createFlow(api, false)
    const wait = versionNodes(flow, "draft").nodes.find((n) => n.type === "wait") as FlowNode
    expect((await api.get(`/v1/analytics/flows/${flow.id}/smart-delay-stats`)).json).toEqual({ [wait.id]: { waiting: 0, sent: 0 } })
    const plain = await api.post<{ id: string }>("/v1/flows", { name: uniqueName("Plain"), folderId: null })
    expect((await api.get(`/v1/analytics/flows/${plain.json.id}/smart-delay-stats`)).json).toEqual({})
  })

  it("is 404 notFound for an unknown or foreign flow", async () => {
    const owner = await tenantApi("an-smart-delay-404")
    const other = await tenantApi("an-smart-delay-404-other")
    const flow = await createFlow(owner.api, false)
    const unknown = await owner.api.get("/v1/analytics/flows/1/smart-delay-stats")
    expectError(unknown, 404, "notFound", { defined: true })
    expect((unknown.json as { message?: string }).message).toBe("Flow does not exists.")
    expectError(await other.api.get(`/v1/analytics/flows/${flow.id}/smart-delay-stats`), 404, "notFound")
  })
})
