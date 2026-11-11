// coupons.createTopic / getTopic / listTopics / updateTopic / archiveTopic /
// unarchiveTopic / deleteTopic (scope `ecommerce`). Workspace time zone is
// UTC, so `expiresAt` is stored as the end of the given UTC day.

import { describe, expect, it } from "vitest"
import { Normalizer } from "../../src/normalize.ts"
import { expectError, expectInvalid, newTenant, qs, UNKNOWN_ID, uniqueName } from "./helpers.ts"
import { bulkCreate, createTopic, endOfUtcDay, TOPIC_KEYS, type Topic } from "./helpers-ecommerce.ts"

type TopicList = { data: Array<Topic & { couponCount: number }>; pageCount: number }

const tomorrow = () => new Date(Date.now() + 24 * 60 * 60 * 1000)
const DAY_MS = 24 * 60 * 60 * 1000

describe("coupons.createTopic", () => {
  it("creates an active topic (200) with the public topic shape", async () => {
    const n = new Normalizer()
    const { tenant, api } = await newTenant("p107-topic-create")
    const name = uniqueName("Summer")
    const created = await api.post<Topic>("/v1/coupon-topics", {
      name: `  ${name}  `,
      description: "  Codes for summer  ",
    })
    expect(created.status, created.text).toBe(200)
    expect(Object.keys(created.json).sort()).toEqual(TOPIC_KEYS)
    expect(n.normalize(created.json, { replace: { [tenant.workspaceId]: "<workspace>" } })).toEqual({
      id: "<id:1>",
      workspaceId: "<workspace>",
      name,
      description: "Codes for summer",
      expiresAt: null,
      status: "active",
      hasEverHadCoupon: false,
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })
    const fetched = await api.get<Topic>(`/v1/coupon-topics/${created.json.id}`)
    expect(fetched.status, fetched.text).toBe(200)
    expect(fetched.json).toEqual(created.json)
  })

  it("stores expiresAt as the end of that day in the workspace time zone; blank description is null", async () => {
    const { api } = await newTenant("p107-topic-expiry")
    const day = tomorrow()
    const created = await createTopic(api, { expiresAt: day.toISOString(), description: "   " })
    expect(created.expiresAt).toBe(endOfUtcDay(day))
    expect(created.description).toBeNull()
    // A date-only string is accepted too (coerced to a date).
    const dateOnly = await createTopic(api, { expiresAt: endOfUtcDay(day).slice(0, 10) })
    expect(dateOnly.expiresAt).toBe(endOfUtcDay(day))
  })

  it("refuses a validity day in the past, a duplicate name (any case) and bad input", async () => {
    const { api } = await newTenant("p107-topic-create-errors")
    const past = new Date(Date.now() - 2 * DAY_MS).toISOString()
    expectError(await api.post("/v1/coupon-topics", { name: uniqueName("Past"), expiresAt: past }), 400, "couponTopicValidityInPast", {
      message: "Coupon topic validity cannot be in the past",
    })
    const topic = await createTopic(api)
    expectError(await api.post("/v1/coupon-topics", { name: topic.name.toUpperCase() }), 400, "couponTopicNameDuplicated", {
      message: "Coupon topic name already exists",
    })
    expectInvalid(await api.post("/v1/coupon-topics", {}), "name")
    expectInvalid(await api.post("/v1/coupon-topics", { name: "   " }), "name")
    expectInvalid(await api.post("/v1/coupon-topics", { name: "x".repeat(256) }), "name")
    expectInvalid(await api.post("/v1/coupon-topics", { name: "ok", description: "d".repeat(1001) }), "description")
    expectInvalid(await api.post("/v1/coupon-topics", { name: "ok", expiresAt: "not a date" }), "expiresAt")
  })
})

describe("coupons.getTopic", () => {
  it("404 for an unknown id, a topic of another workspace and a deleted topic; 422 for a non-numeric id", async () => {
    const { api } = await newTenant("p107-topic-get")
    const other = await newTenant("p107-topic-get-other")
    const foreign = await createTopic(other.api)
    expectError(await api.get(`/v1/coupon-topics/${UNKNOWN_ID}`), 404, "notFound", { message: "Coupon topic not found" })
    expectError(await api.get(`/v1/coupon-topics/${foreign.id}`), 404, "notFound", { message: "Coupon topic not found" })
    expectInvalid(await api.get("/v1/coupon-topics/abc"), "id")
    const topic = await createTopic(api)
    expect((await api.post(`/v1/coupon-topics/${topic.id}/archive`, {})).status).toBe(200)
    expect((await api.delete(`/v1/coupon-topics/${topic.id}`)).status).toBe(200)
    expectError(await api.get(`/v1/coupon-topics/${topic.id}`), 404, "notFound")
  })
})

describe("coupons.listTopics", () => {
  it("lists active topics by name with couponCount; archived=true lists archived ones; search; paging", async () => {
    const { tenant, api } = await newTenant("p107-topic-list")
    const charlie = await createTopic(api, { name: "Charlie topic" })
    const alpha = await createTopic(api, { name: "Alpha topic" })
    const bravo = await createTopic(api, { name: "Bravo topic" })
    await bulkCreate(api, bravo.id, ["BR-1", "BR-2", "BR-3"])
    const archived = await createTopic(api, { name: "Delta archived" })
    expect((await api.post(`/v1/coupon-topics/${archived.id}/archive`, {})).status).toBe(200)

    const all = await api.get<TopicList>("/v1/coupon-topics")
    expect(all.status, all.text).toBe(200)
    expect(all.json.pageCount).toBe(1)
    // `ORDER BY name` (same leading case, so no collation dependence).
    expect(all.json.data.map((topic) => topic.name)).toEqual(["Alpha topic", "Bravo topic", "Charlie topic"])
    expect(all.json.data.map((topic) => topic.couponCount)).toEqual([0, 3, 0])
    const [first] = all.json.data
    expect(Object.keys(first ?? {}).sort()).toEqual([...TOPIC_KEYS, "couponCount"].sort())
    expect(first).toMatchObject({ id: alpha.id, workspaceId: tenant.workspaceId, status: "active", hasEverHadCoupon: false })
    expect(all.json.data[1]).toMatchObject({ id: bravo.id, hasEverHadCoupon: true })
    expect(all.json.data[2]?.id).toBe(charlie.id)

    const onlyArchived = await api.get<TopicList>(`/v1/coupon-topics${qs({ archived: true })}`)
    expect(onlyArchived.status, onlyArchived.text).toBe(200)
    expect(onlyArchived.json.data.map((topic) => [topic.id, topic.status])).toEqual([[archived.id, "archived"]])

    const searched = await api.get<TopicList>(`/v1/coupon-topics${qs({ search: "BRAVO" })}`)
    expect(searched.json.data.map((topic) => topic.id)).toEqual([bravo.id])

    const page2 = await api.get<TopicList>(`/v1/coupon-topics${qs({ page: 2, perPage: 2 })}`)
    expect(page2.status, page2.text).toBe(200)
    expect(page2.json).toMatchObject({ pageCount: 2 })
    expect(page2.json.data.map((topic) => topic.id)).toEqual([charlie.id])

    expectInvalid(await api.get(`/v1/coupon-topics${qs({ perPage: 51 })}`), "perPage")
    expectInvalid(await api.get(`/v1/coupon-topics${qs({ page: 0 })}`), "page")
  })

  it("is empty for a new workspace and never shows another workspace's topics", async () => {
    const { api } = await newTenant("p107-topic-list-empty")
    const other = await newTenant("p107-topic-list-other")
    await createTopic(other.api)
    const response = await api.get<TopicList>("/v1/coupon-topics")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ data: [], pageCount: 0 })
  })
})

describe("coupons.updateTopic", () => {
  it("replaces name, description and expiresAt; a past day is allowed on update; null clears expiresAt", async () => {
    const { api } = await newTenant("p107-topic-update")
    const topic = await createTopic(api, { description: "old", expiresAt: tomorrow().toISOString() })
    const newName = uniqueName("Renamed")
    const past = new Date(Date.now() - 3 * DAY_MS)
    const updated = await api.put<Topic>(`/v1/coupon-topics/${topic.id}`, {
      name: newName,
      description: "new",
      expiresAt: past.toISOString(),
    })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).toMatchObject({
      id: topic.id,
      name: newName,
      description: "new",
      expiresAt: endOfUtcDay(past),
      status: "active",
      createdAt: topic.createdAt,
    })
    expect(Object.keys(updated.json).sort()).toEqual(TOPIC_KEYS)

    // Omitted description / expiresAt stay; null clears.
    const kept = await api.put<Topic>(`/v1/coupon-topics/${topic.id}`, { name: newName })
    expect(kept.json).toMatchObject({ description: "new", expiresAt: endOfUtcDay(past) })
    const cleared = await api.put<Topic>(`/v1/coupon-topics/${topic.id}`, { name: newName, description: null, expiresAt: null })
    expect(cleared.json).toMatchObject({ description: null, expiresAt: null })
    expect((await api.get<Topic>(`/v1/coupon-topics/${topic.id}`)).json).toEqual(cleared.json)
  })

  it("keeps name and description once the topic ever had a coupon (no error)", async () => {
    const { api } = await newTenant("p107-topic-update-locked")
    const topic = await createTopic(api, { description: "fixed" })
    await bulkCreate(api, topic.id, ["LOCK-1"])
    const day = tomorrow()
    const updated = await api.put<Topic>(`/v1/coupon-topics/${topic.id}`, {
      name: uniqueName("Ignored"),
      description: "ignored",
      expiresAt: day.toISOString(),
    })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).toMatchObject({
      name: topic.name,
      description: "fixed",
      expiresAt: endOfUtcDay(day),
      hasEverHadCoupon: true,
    })
  })

  it("404 unknown / foreign topic; duplicate name of another topic; 422 input", async () => {
    const { api } = await newTenant("p107-topic-update-errors")
    const other = await newTenant("p107-topic-update-other")
    const foreign = await createTopic(other.api)
    const first = await createTopic(api)
    const second = await createTopic(api)
    expectError(await api.put(`/v1/coupon-topics/${UNKNOWN_ID}`, { name: "x" }), 404, "notFound", {
      message: "Coupon topic not found",
    })
    expectError(await api.put(`/v1/coupon-topics/${foreign.id}`, { name: "x" }), 404, "notFound")
    expectError(await api.put(`/v1/coupon-topics/${second.id}`, { name: first.name.toLowerCase() }), 400, "couponTopicNameDuplicated")
    // Its own name (another case) is not a duplicate.
    const same = await api.put<Topic>(`/v1/coupon-topics/${second.id}`, { name: second.name.toUpperCase() })
    expect(same.status, same.text).toBe(200)
    expect(same.json.name).toBe(second.name.toUpperCase())
    expectInvalid(await api.put(`/v1/coupon-topics/${first.id}`, {}), "name")
  })
})

describe("coupons.archiveTopic / unarchiveTopic / deleteTopic", () => {
  it("archive -> archived, unarchive -> active (both idempotent); delete takes an archived topic only", async () => {
    const { api } = await newTenant("p107-topic-archive")
    const topic = await createTopic(api)
    const archived = await api.post<Topic>(`/v1/coupon-topics/${topic.id}/archive`, {})
    expect(archived.status, archived.text).toBe(200)
    expect(archived.json).toMatchObject({ id: topic.id, status: "archived", name: topic.name })
    expect(Object.keys(archived.json).sort()).toEqual(TOPIC_KEYS)
    expect((await api.post<Topic>(`/v1/coupon-topics/${topic.id}/archive`, {})).json.status).toBe("archived")

    const active = await api.post<Topic>(`/v1/coupon-topics/${topic.id}/unarchive`, {})
    expect(active.status, active.text).toBe(200)
    expect(active.json).toMatchObject({ id: topic.id, status: "active" })
    expect((await api.post<Topic>(`/v1/coupon-topics/${topic.id}/unarchive`, {})).json.status).toBe("active")

    // An active topic is not deletable.
    expectError(await api.delete(`/v1/coupon-topics/${topic.id}`), 404, "notFound", {
      message: "Archived coupon topic not found",
    })
    await api.post(`/v1/coupon-topics/${topic.id}/archive`, {})
    const deleted = await api.delete<Topic>(`/v1/coupon-topics/${topic.id}`)
    expect(deleted.status, deleted.text).toBe(200)
    expect(deleted.json).toMatchObject({ id: topic.id, status: "archived", name: topic.name })
    expect(Object.keys(deleted.json).sort()).toEqual(TOPIC_KEYS)
    // Gone everywhere; a second delete, archive and unarchive are 404.
    expectError(await api.delete(`/v1/coupon-topics/${topic.id}`), 404, "notFound")
    expectError(await api.post(`/v1/coupon-topics/${topic.id}/archive`, {}), 404, "notFound", { message: "Coupon topic not found" })
    expectError(await api.post(`/v1/coupon-topics/${topic.id}/unarchive`, {}), 404, "notFound")
    const archivedList = await api.get<TopicList>(`/v1/coupon-topics${qs({ archived: true })}`)
    expect(archivedList.json.data).toEqual([])
    // The name is free again.
    const again = await api.post<Topic>("/v1/coupon-topics", { name: topic.name })
    expect(again.status, again.text).toBe(200)
  })

  it("404 for unknown and foreign topics", async () => {
    const { api } = await newTenant("p107-topic-archive-errors")
    const other = await newTenant("p107-topic-archive-other")
    const foreign = await createTopic(other.api)
    for (const path of ["archive", "unarchive"]) {
      expectError(await api.post(`/v1/coupon-topics/${UNKNOWN_ID}/${path}`, {}), 404, "notFound")
      expectError(await api.post(`/v1/coupon-topics/${foreign.id}/${path}`, {}), 404, "notFound")
    }
    await other.api.post(`/v1/coupon-topics/${foreign.id}/archive`, {})
    expectError(await api.delete(`/v1/coupon-topics/${foreign.id}`), 404, "notFound")
    expect((await other.api.get<Topic>(`/v1/coupon-topics/${foreign.id}`)).json.status).toBe("archived")
  })
})
