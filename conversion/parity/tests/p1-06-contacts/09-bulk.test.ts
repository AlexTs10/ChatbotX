// contacts.bulkAddTags / bulkRemoveTags / bulkDelete /
// bulkSubscribeSequences / bulkUnsubscribeSequences / bulkTagByStats.

import { beforeAll, describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import {
  type Api,
  contactPath,
  createSequence,
  expectError,
  expectInvalid,
  ext,
  idOf,
  newTenant,
  seedContact,
  uniqueName,
} from "./helpers.ts"

let tenant: Tenant
let api: Api
let foreignContact: string

beforeAll(async () => {
  ;({ tenant, api } = await newTenant("p106-bulk"))
  const other = await newTenant("p106-bulk-other")
  foreignContact = (await seedContact(other.tenant.workspaceId, { firstName: "Foreign" })).contactId
})

const contacts = async (count: number) => {
  const created: string[] = []
  for (let index = 0; index < count; index += 1) {
    created.push((await seedContact(tenant.workspaceId, { firstName: `Bulk ${index}` })).contactId)
  }
  return created
}
const tagNames = async (contactId: string) => {
  const response = await api.get<{ data: Array<{ name: string }> }>(contactPath(idOf(contactId), "/tags"))
  expect(response.status, response.text).toBe(200)
  return response.json.data.map((tag) => tag.name)
}
const sequenceNames = async (contactId: string) => {
  const response = await api.get<{ data: Array<{ sequenceName: string }> }>(contactPath(idOf(contactId), "/sequences"))
  return response.json.data.map((row) => row.sequenceName).sort()
}

describe("bulkAddTags / bulkRemoveTags", () => {
  it("tags every resolvable contact; unknown and foreign ids are skipped and reported", async () => {
    const [a, b] = (await contacts(2)) as [string, string]
    const response = await api.post("/v1/contacts/bulk/tags", {
      contactIds: [a, "1", b, foreignContact],
      tags: ["bulk-x", " bulk-y "],
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ processed: 2, skippedContactIds: ["1", foreignContact] })
    expect(await tagNames(a)).toEqual(["bulk-x", "bulk-y"])
    expect(await tagNames(b)).toEqual(["bulk-x", "bulk-y"])
    // Again: idempotent, still counted as processed.
    expect((await api.post("/v1/contacts/bulk/tags", { contactIds: [a], tags: ["bulk-x"] })).json).toEqual({
      processed: 1,
      skippedContactIds: [],
    })

    const removed = await api.post("/v1/contacts/bulk/tags/remove", {
      contactIds: [a, b, "1"],
      tags: ["bulk-x", "no-such-tag"],
    })
    expect(removed.status, removed.text).toBe(200)
    expect(removed.json).toEqual({ processed: 2, skippedContactIds: ["1"] })
    expect(await tagNames(a)).toEqual(["bulk-y"])
    // Names that match no tag: the contacts still count as processed.
    expect((await api.post("/v1/contacts/bulk/tags/remove", { contactIds: [a], tags: ["nothing"] })).json).toEqual({
      processed: 1,
      skippedContactIds: [],
    })
  })

  it("validation", async () => {
    for (const route of ["/v1/contacts/bulk/tags", "/v1/contacts/bulk/tags/remove"]) {
      expectInvalid(await api.post(route, { contactIds: [], tags: ["x"] }), "contactIds")
      expectInvalid(await api.post(route, { contactIds: ["abc"], tags: ["x"] }), "contactIds.0")
      expectInvalid(
        await api.post(route, { contactIds: Array.from({ length: 1001 }, (_, i) => String(i + 1)), tags: ["x"] }),
        "contactIds",
      )
      expectInvalid(await api.post(route, { contactIds: ["1"], tags: [] }), "tags")
      expectInvalid(await api.post(route, { contactIds: ["1"], tags: Array.from({ length: 21 }, (_, i) => `t${i}`) }), "tags")
      expectInvalid(await api.post(route, { contactIds: ["1"], tags: [" "] }), "tags.0")
    }
  })
})

describe("bulkDelete", () => {
  it("deletes the resolvable contacts and reports the rest", async () => {
    const [a, b, keep] = (await contacts(3)) as [string, string, string]
    const response = await api.post("/v1/contacts/bulk/delete", { contactIds: [a, b, "1", foreignContact] })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ processed: 2, skippedContactIds: ["1", foreignContact] })
    expectError(await api.get(contactPath(idOf(a))), 404, "notFound")
    expectError(await api.get(contactPath(idOf(b))), 404, "notFound")
    expect((await api.get(contactPath(idOf(keep)))).status).toBe(200)
    expect((await api.post("/v1/contacts/bulk/delete", { contactIds: [a] })).json).toEqual({
      processed: 0,
      skippedContactIds: [a],
    })
    expectInvalid(await api.post("/v1/contacts/bulk/delete", { contactIds: [] }), "contactIds")
  })
})

describe("bulkSubscribeSequences / bulkUnsubscribeSequences", () => {
  it("subscribes and unsubscribes in bulk; an unknown sequence is 404 for both", async () => {
    const [a, b] = (await contacts(2)) as [string, string]
    const s1 = await createSequence(api, "bulk-s1")
    const s2 = await createSequence(api, "bulk-s2")
    const subscribed = await api.post("/v1/contacts/bulk/sequences", { contactIds: [a, b, "1"], sequenceIds: [s1, s2] })
    expect(subscribed.status, subscribed.text).toBe(200)
    expect(subscribed.json).toEqual({ processed: 2, skippedContactIds: ["1"] })
    expect(await sequenceNames(a)).toEqual(["bulk-s1", "bulk-s2"])
    expect(await sequenceNames(b)).toEqual(["bulk-s1", "bulk-s2"])

    const removed = await api.post("/v1/contacts/bulk/sequences/remove", { contactIds: [a, foreignContact], sequenceIds: [s1] })
    expect(removed.status, removed.text).toBe(200)
    expect(removed.json).toEqual({ processed: 1, skippedContactIds: [foreignContact] })
    expect(await sequenceNames(a)).toEqual(["bulk-s2"])

    expectError(await api.post("/v1/contacts/bulk/sequences", { contactIds: [a], sequenceIds: ["1"] }), 404, "notFound", {
      message: "Sequence not found",
    })
    expectError(
      await api.post("/v1/contacts/bulk/sequences/remove", { contactIds: [a], sequenceIds: ["1"] }),
      404,
      "notFound",
      { message: "Sequence not found" },
    )
    expectInvalid(await api.post("/v1/contacts/bulk/sequences", { contactIds: [a], sequenceIds: [] }), "sequenceIds")
    expectInvalid(
      await api.post("/v1/contacts/bulk/sequences/remove", {
        contactIds: [a],
        sequenceIds: Array.from({ length: 21 }, (_, i) => String(i + 1)),
      }),
      "sequenceIds",
    )
  })
})

describe("bulkTagByStats", () => {
  it("sequenceStep: queues a job (202) that tags the contacts behind the step event, minus the excluded ones", async () => {
    const fx = await ext()
    const sequence = await api.post<{ sequenceId: string }>("/v1/sequences", { name: uniqueName("stats") })
    expect(sequence.status, sequence.text).toBe(201)
    const sequenceId = sequence.json.sequenceId
    const step = await api.put<{ stepId: string }>(`/v1/sequences/${sequenceId}/steps`, {
      order: 0,
      delayDays: 0,
      delayMinutes: 0,
      delayUnit: "immediate",
    })
    expect(step.status, step.text).toBe(200)
    const stepId = step.json.stepId
    const seen = await seedContact(tenant.workspaceId, { firstName: "Seen" })
    const excluded = await seedContact(tenant.workspaceId, { firstName: "Excluded" })
    const deliveredOnly = await seedContact(tenant.workspaceId, { firstName: "Delivered" })
    const now = new Date().toISOString()
    for (const [contact, seenAt] of [
      [seen, now],
      [excluded, now],
      [deliveredOnly, null],
    ] as const) {
      await fx.insertSequenceDispatch({
        workspaceId: tenant.workspaceId,
        sequenceId,
        stepId,
        contactId: contact.contactId,
        contactInboxId: contact.contactInboxIds[0] as string,
        deliveredAt: now,
        seenAt,
      })
    }
    const tag = uniqueName("Engaged")
    const response = await api.post("/v1/contacts/bulk/tags/by-stats", {
      source: "sequenceStep",
      sequenceId,
      stepId,
      eventType: "message:seen",
      tags: [tag],
      excludedContactIds: [excluded.contactId],
    })
    expect(response.status, response.text).toBe(202)
    expect(response.json).toEqual({ queued: true })
    await eventually(async () => expect(await tagNames(seen.contactId)).toEqual([tag]), {
      timeoutMs: 20_000,
      intervalMs: 300,
      description: "bulk tag job",
    })
    expect(await tagNames(excluded.contactId)).toEqual([])
    expect(await tagNames(deliveredOnly.contactId)).toEqual([])
  })

  it("commentAutomation: 202 even for an unknown automation; the tags are created at once", async () => {
    const tag = uniqueName("Commenter")
    const response = await api.post("/v1/contacts/bulk/tags/by-stats", {
      source: "commentAutomation",
      automationId: "1",
      eventType: "comment:missed",
      tags: [tag],
    })
    expect(response.status, response.text).toBe(202)
    expect(response.json).toEqual({ queued: true })
    const tags = await api.get<{ data: Array<{ name: string }> }>(`/v1/tags?name=${encodeURIComponent(tag)}`)
    expect(tags.json.data.map((row) => row.name)).toContain(tag)
  })

  it("errors: unknown broadcast / sequence 404; validation", async () => {
    expectError(
      await api.post("/v1/contacts/bulk/tags/by-stats", { source: "broadcast", broadcastId: "1", eventType: "message:seen", tags: ["x"] }),
      404,
      "notFound",
      { message: "Broadcast not found" },
    )
    expectError(
      await api.post("/v1/contacts/bulk/tags/by-stats", {
        source: "sequenceStep",
        sequenceId: "1",
        stepId: "1",
        eventType: "message:seen",
        tags: ["x"],
      }),
      404,
      "notFound",
      { message: "Sequence not found" },
    )
    const base = { source: "broadcast", broadcastId: "1", eventType: "message:seen", tags: ["x"] }
    expectInvalid(await api.post("/v1/contacts/bulk/tags/by-stats", { ...base, source: "poll" }), "source")
    expectInvalid(await api.post("/v1/contacts/bulk/tags/by-stats", { ...base, eventType: "message:received" }), "eventType")
    expectInvalid(await api.post("/v1/contacts/bulk/tags/by-stats", { ...base, eventType: "comment:missed" }), "eventType")
    expectInvalid(await api.post("/v1/contacts/bulk/tags/by-stats", { ...base, tags: [] }), "tags")
    expectInvalid(await api.post("/v1/contacts/bulk/tags/by-stats", { ...base, broadcastId: undefined }), "broadcastId")
    expectInvalid(await api.post("/v1/contacts/bulk/tags/by-stats", { ...base, excludedContactIds: ["x"] }), "excludedContactIds.0")
  })
})
