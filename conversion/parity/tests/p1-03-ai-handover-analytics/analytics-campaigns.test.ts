// analytics.broadcastStats and analytics.sequenceStepStats: delivery
// counters of a broadcast's recipients / a sequence step's dispatches.
// `message:sent` = delivered + failed. An unknown or foreign id answers
// zeros (no 404).

import { describe, expect, it } from "vitest"
import { type Api, ext, tenantApi, uniqueName } from "./helpers.ts"

const ZERO = { "message:sent": 0, "message:delivered": 0, "message:seen": 0, "flow:clicked": 0, "message:failed": 0 }

const contacts = async (workspaceId: string, count: number) => {
  const fx = await ext()
  const { inboxId } = await fx.createInbox({ workspaceId, name: uniqueName("Inbox"), channel: "messenger" })
  return Promise.all(Array.from({ length: count }, (_, i) => fx.createContact({ workspaceId, inboxId, channel: "messenger", firstName: `R${i}` })))
}

/** Four outcomes: delivered+seen+clicked, delivered, failed, nothing yet. */
const OUTCOMES = [
  { deliveredAt: "2025-06-01T10:00:00Z", seenAt: "2025-06-01T10:05:00Z", clickedAt: "2025-06-01T10:06:00Z" },
  { deliveredAt: "2025-06-01T10:00:01Z" },
  { failedAt: "2025-06-01T10:00:02Z", errorContent: '{"code":551}' },
  {},
]

describe("analytics.broadcastStats", () => {
  it("counts the recipients' outcomes", async () => {
    const { tenant, api } = await tenantApi("an-broadcast")
    const fx = await ext()
    const { broadcastId } = await fx.createBroadcast({ workspaceId: tenant.workspaceId, name: uniqueName("Promo"), channel: "messenger" })
    const recipients = await contacts(tenant.workspaceId, OUTCOMES.length)
    await fx.insertBroadcastRecipients(
      recipients.map((r, i) => ({ broadcastId, contactId: r.contactId, contactInboxId: r.contactInboxId, conversationId: r.conversationId, ...OUTCOMES[i] })),
    )
    const response = await api.get(`/v1/analytics/broadcasts/${broadcastId}/stats`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ "message:sent": 3, "message:delivered": 2, "message:seen": 1, "flow:clicked": 1, "message:failed": 1 })
  })

  it("answers zeros for an unknown, an empty and another workspace's broadcast", async () => {
    const owner = await tenantApi("an-broadcast-owner")
    const other = await tenantApi("an-broadcast-other")
    const fx = await ext()
    const { broadcastId } = await fx.createBroadcast({ workspaceId: owner.tenant.workspaceId, name: uniqueName("B"), channel: "messenger" })
    const [r] = await contacts(owner.tenant.workspaceId, 1)
    if (!r) throw new Error("no recipient")
    await fx.insertBroadcastRecipients([{ broadcastId, contactId: r.contactId, contactInboxId: r.contactInboxId, conversationId: r.conversationId, deliveredAt: "2025-06-01T00:00:00Z" }])
    const { broadcastId: empty } = await fx.createBroadcast({ workspaceId: owner.tenant.workspaceId, name: uniqueName("E"), channel: "messenger" })

    expect((await owner.api.get(`/v1/analytics/broadcasts/${empty}/stats`)).json).toEqual(ZERO)
    expect((await owner.api.get("/v1/analytics/broadcasts/1/stats")).json).toEqual(ZERO)
    expect((await other.api.get(`/v1/analytics/broadcasts/${broadcastId}/stats`)).json).toEqual(ZERO)
    expect((await owner.api.get(`/v1/analytics/broadcasts/${broadcastId}/stats`)).json).toEqual({ ...ZERO, "message:sent": 1, "message:delivered": 1 })
  })
})

describe("analytics.sequenceStepStats", () => {
  const sequenceWithStep = async (api: Api) => {
    const sequence = await api.post<{ sequenceId: string }>("/v1/sequences", { name: uniqueName("Drip") })
    expect(sequence.status).toBe(201)
    const step = await api.put<{ stepId: string }>(`/v1/sequences/${sequence.json.sequenceId}/steps`, {
      order: 0,
      delayDays: 0,
      delayMinutes: 0,
      delayUnit: "immediate",
    })
    expect(step.status).toBe(200)
    return { sequenceId: sequence.json.sequenceId, stepId: step.json.stepId }
  }

  it("counts the step's dispatch outcomes", async () => {
    const { tenant, api } = await tenantApi("an-sequence")
    const { sequenceId, stepId } = await sequenceWithStep(api)
    const other = await api.put<{ stepId: string }>(`/v1/sequences/${sequenceId}/steps`, { order: 1, delayDays: 0, delayMinutes: 0, delayUnit: "immediate" })
    const recipients = await contacts(tenant.workspaceId, OUTCOMES.length + 1)
    const fx = await ext()
    await fx.insertSequenceDispatches([
      ...OUTCOMES.map((outcome, i) => ({
        workspaceId: tenant.workspaceId,
        sequenceId,
        stepId,
        contactId: recipients[i]?.contactId as string,
        contactInboxId: recipients[i]?.contactInboxId as string,
        ...outcome,
      })),
      // the other step's dispatch is not counted
      {
        workspaceId: tenant.workspaceId,
        sequenceId,
        stepId: other.json.stepId,
        contactId: recipients[4]?.contactId as string,
        contactInboxId: recipients[4]?.contactInboxId as string,
        deliveredAt: "2025-06-02T00:00:00Z",
      },
    ])
    const response = await api.get(`/v1/analytics/sequences/${sequenceId}/steps/${stepId}/stats`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ "message:sent": 3, "message:delivered": 2, "message:seen": 1, "flow:clicked": 1, "message:failed": 1 })
    expect((await api.get(`/v1/analytics/sequences/${sequenceId}/steps/${other.json.stepId}/stats`)).json).toEqual({
      ...ZERO,
      "message:sent": 1,
      "message:delivered": 1,
    })
  })

  it("answers zeros for an unknown step and another workspace's sequence", async () => {
    const owner = await tenantApi("an-sequence-owner")
    const other = await tenantApi("an-sequence-other")
    const { sequenceId, stepId } = await sequenceWithStep(owner.api)
    const [r] = await contacts(owner.tenant.workspaceId, 1)
    if (!r) throw new Error("no recipient")
    await (await ext()).insertSequenceDispatches([
      { workspaceId: owner.tenant.workspaceId, sequenceId, stepId, contactId: r.contactId, contactInboxId: r.contactInboxId, failedAt: "2025-06-01T00:00:00Z" },
    ])
    expect((await owner.api.get(`/v1/analytics/sequences/${sequenceId}/steps/1/stats`)).json).toEqual(ZERO)
    expect((await other.api.get(`/v1/analytics/sequences/${sequenceId}/steps/${stepId}/stats`)).json).toEqual(ZERO)
    expect((await owner.api.get(`/v1/analytics/sequences/${sequenceId}/steps/${stepId}/stats`)).json).toEqual({
      ...ZERO,
      "message:sent": 1,
      "message:failed": 1,
    })
  })
})
