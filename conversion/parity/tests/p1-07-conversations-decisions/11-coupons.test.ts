// coupons.bulkCreate / listCoupons / issueCoupon / markCouponUsed /
// listContactCoupons (scope `ecommerce`). Contacts come from the public
// webchat + contacts.create routes.

import { describe, expect, it } from "vitest"
import { Normalizer } from "../../src/normalize.ts"
import {
  contactPath,
  createWebchat,
  createWebchatContact,
  expectError,
  expectInvalid,
  idOf,
  newTenant,
  qs,
  sortBy,
  UNKNOWN_ID,
} from "./helpers.ts"
import {
  type BulkResult,
  bulkCreate,
  createTopic,
  type IssuedCoupon,
  type ListedCoupon,
  newContact,
  newContacts,
  type Topic,
} from "./helpers-ecommerce.ts"

type CouponList = { data: ListedCoupon[]; pageCount: number }
type ContactCoupon = { id: string; topicId: string; topicName: string; code: string; usedAt: string | null; issuedAt: string | null }

const ISSUED_KEYS = ["code", "createdAt", "id", "issuedAt", "issuedContactId", "topicId", "updatedAt", "usedAt", "workspaceId"]
const LISTED_KEYS = [...ISSUED_KEYS, "issueStatus", "topicName", "usageStatus"].sort()

describe("coupons.bulkCreate", () => {
  it("adds trimmed, de-duplicated codes; codes already in the workspace count as existing", async () => {
    const { api } = await newTenant("p107-bulk")
    const topic = await createTopic(api)
    const first = await api.post<BulkResult>(`/v1/coupon-topics/${topic.id}/coupons/bulk`, {
      codes: [" A-1 ", "A-2", "A-1", "A-3"],
    })
    expect(first.status, first.text).toBe(200)
    expect(first.json).toEqual({ processed: 3, created: 3, existing: 0, allowedRemaining: 10_000, currentCount: 0 })
    expect((await api.get<Topic>(`/v1/coupon-topics/${topic.id}`)).json.hasEverHadCoupon).toBe(true)

    const second = await bulkCreate(api, topic.id, ["A-3", "A-4"])
    expect(second).toEqual({ processed: 2, created: 1, existing: 1, allowedRemaining: 9_997, currentCount: 3 })

    // Codes are unique per workspace, across topics.
    const otherTopic = await createTopic(api)
    expect(await bulkCreate(api, otherTopic.id, ["A-1", "B-1"])).toEqual({
      processed: 2,
      created: 1,
      existing: 1,
      allowedRemaining: 10_000,
      currentCount: 0,
    })
    // ... but not across workspaces.
    const other = await newTenant("p107-bulk-other")
    const foreignTopic = await createTopic(other.api)
    expect((await bulkCreate(other.api, foreignTopic.id, ["A-1"])).created).toBe(1)

    const listed = await api.get<CouponList>(`/v1/coupons${qs({ topicId: topic.id })}`)
    expect(sortBy(listed.json.data, (c) => c.code).map((c) => c.code)).toEqual(["A-1", "A-2", "A-3", "A-4"])
  })

  it("refuses an archived, unknown or foreign topic (400 couponTopicInactive) and bad input", async () => {
    const { api } = await newTenant("p107-bulk-errors")
    const other = await newTenant("p107-bulk-errors-other")
    const foreign = await createTopic(other.api)
    const topic = await createTopic(api)
    await api.post(`/v1/coupon-topics/${topic.id}/archive`, {})
    const inactive = { message: "Coupon topic is not active" }
    expectError(await api.post(`/v1/coupon-topics/${topic.id}/coupons/bulk`, { codes: ["X1"] }), 400, "couponTopicInactive", inactive)
    expectError(await api.post(`/v1/coupon-topics/${UNKNOWN_ID}/coupons/bulk`, { codes: ["X1"] }), 400, "couponTopicInactive", inactive)
    expectError(await api.post(`/v1/coupon-topics/${foreign.id}/coupons/bulk`, { codes: ["X1"] }), 400, "couponTopicInactive", inactive)
    const active = await createTopic(api)
    const path = `/v1/coupon-topics/${active.id}/coupons/bulk`
    expectInvalid(await api.post(path, { codes: [] }), "codes")
    expectInvalid(await api.post(path, {}), "codes")
    expectInvalid(await api.post(path, { codes: ["  "] }), "codes.0")
    expectInvalid(await api.post(path, { codes: ["ok", "c".repeat(256)] }), "codes.1")
    expectInvalid(await api.post(path, { codes: Array.from({ length: 10_001 }, (_, i) => `C${i}`) }), "codes")
  })

  it("enforces the 10 000 coupons per topic limit (400 couponImportLimitExceeded, nothing added)", async () => {
    const { api } = await newTenant("p107-bulk-limit")
    const topic = await createTopic(api)
    const full = await bulkCreate(
      api,
      topic.id,
      Array.from({ length: 10_000 }, (_, i) => `L${i}`),
    )
    expect(full).toEqual({ processed: 10_000, created: 10_000, existing: 0, allowedRemaining: 10_000, currentCount: 0 })
    expectError(
      await api.post(`/v1/coupon-topics/${topic.id}/coupons/bulk`, { codes: ["L-new"] }),
      400,
      "couponImportLimitExceeded",
      { message: "Coupon import exceeds topic limit" },
    )
    // Codes that all exist already pass (nothing new to add).
    expect(await bulkCreate(api, topic.id, ["L0"])).toEqual({
      processed: 1,
      created: 0,
      existing: 1,
      allowedRemaining: 0,
      currentCount: 10_000,
    })
    const page = await api.get<CouponList>(`/v1/coupons${qs({ topicId: topic.id, perPage: 50 })}`)
    expect(page.json.pageCount).toBe(200)
    const listed = await api.get<{ data: Array<{ couponCount: number }> }>("/v1/coupon-topics")
    expect(listed.json.data.map((t) => t.couponCount)).toEqual([10_000])
  })
})

describe("coupons.listCoupons", () => {
  it("lists coupons of active topics with topicName, issue and usage status; filters and aliases", async () => {
    const n = new Normalizer()
    const { tenant, api } = await newTenant("p107-list-coupons")
    const topic = await createTopic(api, { name: "Main topic" })
    await bulkCreate(api, topic.id, ["MAIN-ONE", "MAIN-TWO", "main-three"])
    const archivedTopic = await createTopic(api, { name: "Old topic" })
    await bulkCreate(api, archivedTopic.id, ["OLD-ONE"])
    await api.post(`/v1/coupon-topics/${archivedTopic.id}/archive`, {})
    const [alice, bob] = await newContacts(api, 2)
    const issuedA = await api.post<IssuedCoupon>(`/v1/coupon-topics/${topic.id}/issue`, { contactId: alice })
    expect(issuedA.status, issuedA.text).toBe(200)
    const issuedB = await api.post<IssuedCoupon>(`/v1/coupon-topics/${topic.id}/issue`, { contactId: bob })
    expect((await api.post(`/v1/coupon-topics/${topic.id}/mark-used`, { contactId: bob })).status).toBe(200)

    const all = await api.get<CouponList>("/v1/coupons")
    expect(all.status, all.text).toBe(200)
    expect(all.json.pageCount).toBe(1)
    // Coupons of the archived topic are not listed.
    const rows = sortBy(all.json.data, (c) => c.code)
    expect(rows.map((c) => c.code)).toEqual(["MAIN-ONE", "MAIN-TWO", "main-three"])
    expect(Object.keys(rows[0] ?? {}).sort()).toEqual(LISTED_KEYS)
    const byCode = Object.fromEntries(rows.map((c) => [c.code, c]))
    const usedCode = issuedB.json.code
    const issuedCode = issuedA.json.code
    // Issue order is by coupon id: the first two codes go out.
    expect([issuedCode, usedCode]).toEqual(["MAIN-ONE", "MAIN-TWO"])
    expect(n.normalize(byCode[issuedCode], { replace: { [tenant.workspaceId]: "<workspace>" } })).toEqual({
      id: "<id:1>",
      workspaceId: "<workspace>",
      topicId: "<id:3>",
      topicName: "Main topic",
      code: issuedCode,
      issuedContactId: "<id:2>",
      issuedAt: "<ts>",
      usedAt: null,
      issueStatus: "published",
      usageStatus: "notUsed",
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })
    expect(byCode[issuedCode]).toMatchObject({ topicId: topic.id, issuedContactId: alice })
    expect(byCode[usedCode]).toMatchObject({ issuedContactId: bob, issueStatus: "published", usageStatus: "used" })
    expect(byCode["main-three"]).toMatchObject({
      issuedContactId: null,
      issuedAt: null,
      usedAt: null,
      issueStatus: "unpublished",
      usageStatus: "notUsed",
    })

    const codesOf = async (query: Record<string, string | number>) => {
      const response = await api.get<CouponList>(`/v1/coupons${qs(query)}`)
      expect(response.status, response.text).toBe(200)
      return response.json.data.map((c) => c.code).sort()
    }
    expect(await codesOf({ status: "published" })).toEqual(["MAIN-ONE", "MAIN-TWO"])
    expect(await codesOf({ issueStatus: "unpublished" })).toEqual(["main-three"])
    expect(await codesOf({ usage: "used" })).toEqual(["MAIN-TWO"])
    expect(await codesOf({ usageStatus: "notUsed" })).toEqual(["MAIN-ONE", "main-three"])
    expect(await codesOf({ keyword: "MAIN-T" })).toEqual(["MAIN-TWO", "main-three"])
    expect(await codesOf({ search: "one" })).toEqual(["MAIN-ONE"])
    expect(await codesOf({ topicId: archivedTopic.id })).toEqual([])
    expect(await codesOf({ topicId: topic.id, status: "published", usage: "notUsed" })).toEqual(["MAIN-ONE"])

    const paged = await api.get<CouponList>(`/v1/coupons${qs({ perPage: 2, page: 2 })}`)
    expect(paged.json.pageCount).toBe(2)
    expect(paged.json.data).toHaveLength(1)
  })

  it("422 when a filter and its legacy alias are both given, or for bad enums / paging", async () => {
    const { api } = await newTenant("p107-list-coupons-errors")
    expectInvalid(await api.get(`/v1/coupons${qs({ status: "published", issueStatus: "published" })}`), "status")
    expectInvalid(await api.get(`/v1/coupons${qs({ usage: "used", usageStatus: "used" })}`), "usage")
    expectInvalid(await api.get(`/v1/coupons${qs({ keyword: "a", search: "a" })}`), "keyword")
    expectInvalid(await api.get(`/v1/coupons${qs({ status: "issued" })}`), "status")
    expectInvalid(await api.get(`/v1/coupons${qs({ usage: "yes" })}`), "usage")
    expectInvalid(await api.get(`/v1/coupons${qs({ topicId: "abc" })}`), "topicId")
    expectInvalid(await api.get(`/v1/coupons${qs({ perPage: 51 })}`), "perPage")
    const empty = await api.get<CouponList>("/v1/coupons")
    expect(empty.json).toEqual({ data: [], pageCount: 0 })
  })
})

// The handlers throw `new ORPCError(code, {message, data})` without a status:
// oRPC answers 500 with `defined: false` although the route documents 409 /
// 404. The Rust port must answer the same.
const UNAVAILABLE = { defined: false, message: "No coupon could be issued for this topic" }

describe("coupons.issueCoupon", () => {
  it("issues the next free coupon; a second call for the same contact returns the same coupon", async () => {
    const n = new Normalizer()
    const { tenant, api } = await newTenant("p107-issue")
    const topic = await createTopic(api)
    await bulkCreate(api, topic.id, ["ISS-1", "ISS-2"])
    const contactId = await newContact(api)
    const issued = await api.post<IssuedCoupon>(`/v1/coupon-topics/${topic.id}/issue`, { contactId })
    expect(issued.status, issued.text).toBe(200)
    expect(Object.keys(issued.json).sort()).toEqual(ISSUED_KEYS)
    expect(n.normalize(issued.json, { replace: { [tenant.workspaceId]: "<workspace>" } })).toEqual({
      id: "<id:1>",
      workspaceId: "<workspace>",
      topicId: "<id:3>",
      code: "ISS-1",
      issuedContactId: "<id:2>",
      issuedAt: "<ts>",
      usedAt: null,
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })
    expect(issued.json).toMatchObject({ topicId: topic.id, issuedContactId: contactId })
    const again = await api.post<IssuedCoupon>(`/v1/coupon-topics/${topic.id}/issue`, { contactId })
    expect(again.status, again.text).toBe(200)
    expect(again.json).toEqual(issued.json)
    const remaining = await api.get<CouponList>(`/v1/coupons${qs({ status: "unpublished" })}`)
    expect(remaining.json.data.map((c) => c.code)).toEqual(["ISS-2"])
  })

  it("500 couponIssueUnavailable (documented 409): noAvailableCoupon when all are issued; topicUnavailable when archived or expired", async () => {
    const { api } = await newTenant("p107-issue-unavailable")
    const [first, second] = await newContacts(api, 2)
    const topic = await createTopic(api)
    await bulkCreate(api, topic.id, ["ONLY-1"])
    expect((await api.post(`/v1/coupon-topics/${topic.id}/issue`, { contactId: first })).status).toBe(200)
    const none = expectError(
      await api.post(`/v1/coupon-topics/${topic.id}/issue`, { contactId: second }),
      500,
      "couponIssueUnavailable",
      UNAVAILABLE,
    )
    expect(none.data).toEqual({ reason: "noAvailableCoupon" })
    // An empty topic is also noAvailableCoupon.
    const empty = await createTopic(api)
    expect(
      expectError(await api.post(`/v1/coupon-topics/${empty.id}/issue`, { contactId: first }), 500, "couponIssueUnavailable", UNAVAILABLE).data,
    ).toEqual({ reason: "noAvailableCoupon" })

    const archived = await createTopic(api)
    await bulkCreate(api, archived.id, ["ARCH-1"])
    await api.post(`/v1/coupon-topics/${archived.id}/archive`, {})
    expect(
      expectError(await api.post(`/v1/coupon-topics/${archived.id}/issue`, { contactId: first }), 500, "couponIssueUnavailable", UNAVAILABLE)
        .data,
    ).toEqual({ reason: "topicUnavailable" })

    const expired = await createTopic(api)
    await bulkCreate(api, expired.id, ["EXP-1"])
    const past = new Date(Date.now() - 3 * 24 * 60 * 60 * 1000).toISOString()
    expect((await api.put(`/v1/coupon-topics/${expired.id}`, { name: expired.name, expiresAt: past })).status).toBe(200)
    expect(
      expectError(await api.post(`/v1/coupon-topics/${expired.id}/issue`, { contactId: first }), 500, "couponIssueUnavailable", UNAVAILABLE)
        .data,
    ).toEqual({ reason: "topicUnavailable" })
    // Unknown topic (with a valid contact) is topicUnavailable too, not 404.
    expect(
      expectError(await api.post(`/v1/coupon-topics/${UNKNOWN_ID}/issue`, { contactId: first }), 500, "couponIssueUnavailable", UNAVAILABLE)
        .data,
    ).toEqual({ reason: "topicUnavailable" })
  })

  it("404 for an unknown or foreign contact (checked first); 422 for bad ids", async () => {
    const { api } = await newTenant("p107-issue-errors")
    const other = await newTenant("p107-issue-errors-other")
    const foreignContact = await newContact(other.api)
    const topic = await createTopic(api)
    await bulkCreate(api, topic.id, ["E-1"])
    expectError(await api.post(`/v1/coupon-topics/${topic.id}/issue`, { contactId: UNKNOWN_ID }), 404, "notFound", {
      message: "Contact not found",
    })
    expectError(await api.post(`/v1/coupon-topics/${topic.id}/issue`, { contactId: foreignContact }), 404, "notFound")
    expectError(await api.post(`/v1/coupon-topics/${UNKNOWN_ID}/issue`, { contactId: UNKNOWN_ID }), 404, "notFound")
    expectInvalid(await api.post(`/v1/coupon-topics/${topic.id}/issue`, {}), "contactId")
    expectInvalid(await api.post(`/v1/coupon-topics/${topic.id}/issue`, { contactId: "id:1" }), "contactId")
    // Nothing was issued.
    const listed = await api.get<CouponList>(`/v1/coupons${qs({ status: "published" })}`)
    expect(listed.json.data).toEqual([])
  })
})

describe("coupons.markCouponUsed", () => {
  it("sets usedAt once; a second call returns the same row", async () => {
    const { api } = await newTenant("p107-mark-used")
    const topic = await createTopic(api)
    await bulkCreate(api, topic.id, ["USE-1"])
    const contactId = await newContact(api)
    const issued = await api.post<IssuedCoupon>(`/v1/coupon-topics/${topic.id}/issue`, { contactId })
    const used = await api.post<IssuedCoupon>(`/v1/coupon-topics/${topic.id}/mark-used`, { contactId })
    expect(used.status, used.text).toBe(200)
    expect(Object.keys(used.json).sort()).toEqual(ISSUED_KEYS)
    expect(used.json).toMatchObject({
      id: issued.json.id,
      code: "USE-1",
      issuedContactId: contactId,
      issuedAt: issued.json.issuedAt,
    })
    expect(typeof used.json.usedAt).toBe("string")
    const again = await api.post<IssuedCoupon>(`/v1/coupon-topics/${topic.id}/mark-used`, { contactId })
    expect(again.status, again.text).toBe(200)
    expect(again.json.usedAt).toBe(used.json.usedAt)
    // An archived topic's coupon can still be marked used (no topic check).
    await api.post(`/v1/coupon-topics/${topic.id}/archive`, {})
    expect((await api.post(`/v1/coupon-topics/${topic.id}/mark-used`, { contactId })).status).toBe(200)
  })

  it("500 couponNotIssued (documented 404) when the contact has no coupon of the topic; 404 unknown contact", async () => {
    const { api } = await newTenant("p107-mark-used-errors")
    const topic = await createTopic(api)
    await bulkCreate(api, topic.id, ["NI-1"])
    const contactId = await newContact(api)
    const body = expectError(
      await api.post(`/v1/coupon-topics/${topic.id}/mark-used`, { contactId }),
      500,
      "couponNotIssued",
      { defined: false, message: "No coupon has been issued to this contact for this topic" },
    )
    expect(body.data).toEqual({ reason: "noIssuedCoupon" })
    expectError(await api.post(`/v1/coupon-topics/${topic.id}/mark-used`, { contactId: UNKNOWN_ID }), 404, "notFound", {
      message: "Contact not found",
    })
    expectInvalid(await api.post(`/v1/coupon-topics/${topic.id}/mark-used`, {}), "contactId")
  })
})

describe("coupons.listContactCoupons", () => {
  it("lists the contact's coupons by topic name (archived topics included, deleted excluded)", async () => {
    const { api } = await newTenant("p107-contact-coupons")
    const contactId = await newContact(api)
    const zulu = await createTopic(api, { name: "Zulu topic" })
    const kilo = await createTopic(api, { name: "Kilo topic" })
    const gone = await createTopic(api, { name: "Gone topic" })
    await bulkCreate(api, zulu.id, ["ZU-1"])
    await bulkCreate(api, kilo.id, ["KI-1"])
    await bulkCreate(api, gone.id, ["GO-1"])
    for (const topic of [zulu, kilo, gone]) {
      expect((await api.post(`/v1/coupon-topics/${topic.id}/issue`, { contactId })).status).toBe(200)
    }
    await api.post(`/v1/coupon-topics/${kilo.id}/mark-used`, { contactId })
    await api.post(`/v1/coupon-topics/${kilo.id}/archive`, {})
    await api.post(`/v1/coupon-topics/${gone.id}/archive`, {})
    expect((await api.delete(`/v1/coupon-topics/${gone.id}`)).status).toBe(200)

    const response = await api.get<{ data: ContactCoupon[] }>(contactPath(idOf(contactId), "/coupons"))
    expect(response.status, response.text).toBe(200)
    expect(response.json.data.map((c) => Object.keys(c).sort())).toEqual([
      ["code", "id", "issuedAt", "topicId", "topicName", "usedAt"],
      ["code", "id", "issuedAt", "topicId", "topicName", "usedAt"],
    ])
    expect(response.json.data).toMatchObject([
      { topicId: kilo.id, topicName: "Kilo topic", code: "KI-1" },
      { topicId: zulu.id, topicName: "Zulu topic", code: "ZU-1", usedAt: null },
    ])
    expect(typeof response.json.data[0]?.usedAt).toBe("string")
    expect(typeof response.json.data[1]?.issuedAt).toBe("string")
  })

  it("resolves email: identifiers; empty list for a contact without coupons; 404 for unknown / bad identifiers", async () => {
    const { api } = await newTenant("p107-contact-coupons-errors")
    const { inboxId } = await createWebchat(api, "Coupons chat")
    const email = `buyer-${Date.now()}@parity.test`
    const contactId = await createWebchatContact(api, inboxId, { firstName: "Mail", email })
    const topic = await createTopic(api)
    await bulkCreate(api, topic.id, ["EM-1"])
    expect((await api.get<{ data: unknown[] }>(contactPath(idOf(contactId), "/coupons"))).json).toEqual({ data: [] })
    await api.post(`/v1/coupon-topics/${topic.id}/issue`, { contactId })
    const byEmail = await api.get<{ data: ContactCoupon[] }>(contactPath(`email:${email}`, "/coupons"))
    expect(byEmail.status, byEmail.text).toBe(200)
    expect(byEmail.json.data.map((c) => c.code)).toEqual(["EM-1"])

    expectError(await api.get(contactPath(idOf(UNKNOWN_ID), "/coupons")), 404, "notFound", { message: "Contact not found" })
    expectError(await api.get(contactPath("nope", "/coupons")), 404, "notFound", { message: "Invalid identifier format" })
    expectError(await api.get(contactPath("name:x", "/coupons")), 404, "notFound")
    const other = await newTenant("p107-contact-coupons-other")
    expectError(await other.api.get(contactPath(idOf(contactId), "/coupons")), 404, "notFound")
  })
})
