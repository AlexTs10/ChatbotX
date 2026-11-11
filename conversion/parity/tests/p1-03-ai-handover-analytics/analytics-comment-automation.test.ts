// Comment automation analytics: commentAutomationReplyStats,
// commentAutomationUserComments, commentAutomationBotReplies,
// commentAutomationErrors, commentAutomationContacts.
//
// The automation (type messenger: private replies are the counted half) and
// its CommentAutomationEvent rows are fixtures. Texts are lowercase and
// differ in their first letter, so the text sort does not depend on the
// database collation.

import { beforeAll, describe, expect, it } from "vitest"
import type { CreatedContact } from "../../src/fixtures/ext/p1-03.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import { type Api, expectError, expectInvalid, ext, tenantApi, uniqueName } from "./helpers.ts"

const UTC_RANGE = { from: "2025-07-01T00:00:00Z", to: "2025-07-03T23:59:59Z", timezone: "UTC" }
const HCM_RANGE = { from: "2025-07-01T00:00:00+07:00", to: "2025-07-03T23:59:59+07:00", timezone: "Asia/Ho_Chi_Minh" }

describe("comment automation analytics over seeded events", () => {
  let tenant: Tenant
  let api: Api
  let automationId: string
  let k: CreatedContact[]
  let ids: string[]

  beforeAll(async () => {
    ;({ tenant, api } = await tenantApi("an-comments"))
    const ws = tenant.workspaceId
    const fx = await ext()
    const { inboxId } = await fx.createInbox({ workspaceId: ws, name: uniqueName("Page"), channel: "messenger" })
    k = []
    for (const [firstName, lastName] of [["Kay", "One"], ["Kay", "Two"], ["Kay", "Three"], ["Kay", "Four"]] as const) {
      k.push(await fx.createContact({ workspaceId: ws, inboxId, channel: "messenger", firstName, lastName }))
    }
    ;({ automationId } = await fx.createCommentAutomation({
      workspaceId: ws,
      inboxId,
      name: uniqueName("Auto"),
      counters: { sentCount: 4, deliveredCount: 3, seenCount: 1, clickedCount: 1, failedCount: 1, missedCount: 0 },
    }))
    const base = (contact: CreatedContact | undefined) => ({
      workspaceId: ws,
      automationId,
      contactId: contact?.contactId ?? null,
      contactInboxId: contact?.contactInboxId ?? null,
      postId: "post-1",
      replyType: "text" as const,
    })
    ;({ ids } = await fx.insertCommentAutomationEvents([
      // E0: private DM to K1
      { ...base(k[0]), commentId: "c1", commentText: "price?", replyChannel: "private", replyText: "dm-1", status: "sent", occurredAt: "2025-07-01T10:00:00Z", deliveredAt: "2025-07-01T10:00:05Z" },
      // E1: public reply to the same comment
      { ...base(k[0]), commentId: "c1", commentText: "price?", replyChannel: "public", replyText: "thanks", status: "sent", occurredAt: "2025-07-01T10:00:01Z" },
      // E2: 07-01 in UTC, 07-02 in Ho Chi Minh City
      { ...base(k[1]), commentId: "c2", commentText: "price?", replyChannel: "private", replyText: "dm-1", status: "sent", occurredAt: "2025-07-01T23:30:00Z", deliveredAt: "2025-07-01T23:31:00Z", seenAt: "2025-07-02T00:00:00Z" },
      // E3: failed DM
      { ...base(k[3]), commentId: "c5", commentText: "how much", replyChannel: "private", status: "failed", occurredAt: "2025-07-02T12:00:00Z", failedAt: "2025-07-02T12:00:00Z", errorDetail: "user blocked the page", httpCode: "400" },
      // E4
      { ...base(k[2]), commentId: "c4", commentText: "what is the price", replyChannel: "private", replyText: "dm-2", status: "sent", occurredAt: "2025-07-03T05:00:00Z", deliveredAt: "2025-07-03T05:00:10Z", clickedAt: "2025-07-03T06:00:00Z" },
      // E5: outside the ranges
      { ...base(k[0]), commentId: "c9", commentText: "old comment", replyChannel: "private", replyText: "dm-old", status: "sent", occurredAt: "2025-05-15T00:00:00Z", deliveredAt: "2025-05-15T00:00:01Z" },
    ]))
  })

  const q = (route: string, query: Record<string, unknown>) => api.get(`/v1/analytics/comment-automation/${route}`, { query: { automationId, ...query } as Record<string, string> })

  describe("analytics.commentAutomationReplyStats", () => {
    it("counts sent replies per local day, zero-filled", async () => {
      const response = await q("replies", UTC_RANGE)
      expect(response.status).toBe(200)
      expect(response.json).toEqual({
        data: [
          { dateReport: "2025-07-01", count: 3 },
          { dateReport: "2025-07-02", count: 0 },
          { dateReport: "2025-07-03", count: 1 },
        ],
      })
      expect((await q("replies", HCM_RANGE)).json).toEqual({
        data: [
          { dateReport: "2025-07-01", count: 2 },
          { dateReport: "2025-07-02", count: 1 },
          { dateReport: "2025-07-03", count: 1 },
        ],
      })
    })

    it("uses monthly buckets past 60 days", async () => {
      const response = await q("replies", { from: "2025-05-01T00:00:00Z", to: "2025-07-31T23:59:59Z", timezone: "UTC" })
      expect(response.json).toEqual({
        data: [
          { dateReport: "2025-05-01", count: 1 },
          { dateReport: "2025-06-01", count: 0 },
          { dateReport: "2025-07-01", count: 4 },
        ],
      })
    })

    it("is empty for an automation of another workspace or an unknown one", async () => {
      const other = await tenantApi("an-comments-other")
      expect((await other.api.get("/v1/analytics/comment-automation/replies", { query: { automationId, ...UTC_RANGE } })).json).toEqual({ data: [] })
      expect((await api.get("/v1/analytics/comment-automation/replies", { query: { automationId: "1", ...UTC_RANGE } })).json).toEqual({ data: [] })
    })
  })

  describe("analytics.commentAutomationUserComments", () => {
    it("groups customer comments by text, counting distinct comments", async () => {
      const response = await q("user-comments", UTC_RANGE)
      expect(response.status).toBe(200)
      expect(response.json).toEqual({
        data: [
          { text: "price?", total: 2 },
          { text: "how much", total: 1 },
          { text: "what is the price", total: 1 },
        ],
        total: 3,
        page: 1,
        pageCount: 1,
      })
    })

    it("filters by keyword (case-insensitive) and pages", async () => {
      expect((await q("user-comments", { ...UTC_RANGE, keyword: "PRICE" })).json).toEqual({
        data: [
          { text: "price?", total: 2 },
          { text: "what is the price", total: 1 },
        ],
        total: 2,
        page: 1,
        pageCount: 1,
      })
      expect((await q("user-comments", { ...UTC_RANGE, perPage: 1, page: 2 })).json).toEqual({
        data: [{ text: "how much", total: 1 }],
        total: 3,
        page: 2,
        pageCount: 3,
      })
    })
  })

  describe("analytics.commentAutomationBotReplies", () => {
    it("groups the sent reply texts", async () => {
      const response = await q("bot-replies", UTC_RANGE)
      expect(response.status).toBe(200)
      expect(response.json).toEqual({
        data: [
          { text: "dm-1", total: 2 },
          { text: "dm-2", total: 1 },
          { text: "thanks", total: 1 },
        ],
        total: 3,
        page: 1,
        pageCount: 1,
      })
      expect((await q("bot-replies", { ...UTC_RANGE, keyword: "zzz" })).json).toEqual({ data: [], total: 0, page: 1, pageCount: 0 })
    })

    it("is an empty page for an unknown automation", async () => {
      expect((await api.get("/v1/analytics/comment-automation/bot-replies", { query: { automationId: "1", ...UTC_RANGE, page: "2" } })).json).toEqual({
        data: [],
        total: 0,
        page: 2,
        pageCount: 0,
      })
    })
  })

  describe("analytics.commentAutomationErrors", () => {
    it("lists failed replies newest first with the contact's name", async () => {
      const response = await q("errors", UTC_RANGE)
      expect(response.status).toBe(200)
      expect(response.json).toEqual({
        data: [
          {
            id: ids[3],
            replyChannel: "private",
            replyType: "text",
            errorDetail: "user blocked the page",
            httpCode: "400",
            commentText: "how much",
            contact: { firstName: "Kay", lastName: "Four", avatar: null },
            occurredAt: "2025-07-02T12:00:00.000Z",
          },
        ],
        total: 1,
        page: 1,
        pageCount: 1,
      })
      expect((await q("errors", { ...UTC_RANGE, keyword: "BLOCKED" })).json).toMatchObject({ total: 1 })
      expect((await q("errors", { ...UTC_RANGE, keyword: "nothing" })).json).toEqual({ data: [], total: 0, page: 1, pageCount: 0 })
    })
  })

  describe("analytics.commentAutomationContacts", () => {
    const row = (eventIndex: number, contact: CreatedContact | undefined, occurredAt: string, errorContent: string | null = null) => ({
      rowKey: ids[eventIndex],
      contactId: contact?.contactId,
      contactInboxId: contact?.contactInboxId,
      sourceId: contact?.sourceId,
      channel: "messenger",
      conversationId: contact?.conversationId,
      errorContent,
      occurredAt,
      commentText: null,
      missReason: null,
    })

    it("lists the private-reply events behind a counter, newest first", async () => {
      const delivered = await api.get("/v1/analytics/comment-automation/contacts", { query: { automationId, eventType: "message:delivered" } })
      expect(delivered.status).toBe(200)
      expect(delivered.json).toEqual({
        data: [
          row(4, k[2], "2025-07-03T05:00:10.000Z"),
          row(2, k[1], "2025-07-01T23:31:00.000Z"),
          row(0, k[0], "2025-07-01T10:00:05.000Z"),
          row(5, k[0], "2025-05-15T00:00:01.000Z"),
        ],
        total: 3,
        contactTotal: 3,
        page: 1,
        pageCount: 1,
      })
    })

    it("covers sent, failed, seen and clicked, and pages", async () => {
      const get = (eventType: string, extra: Record<string, string> = {}) =>
        api.get("/v1/analytics/comment-automation/contacts", { query: { automationId, eventType, ...extra } })
      expect((await get("message:sent")).json).toEqual({
        data: [
          row(4, k[2], "2025-07-03T05:00:10.000Z"),
          row(3, k[3], "2025-07-02T12:00:00.000Z", "user blocked the page"),
          row(2, k[1], "2025-07-01T23:31:00.000Z"),
          row(0, k[0], "2025-07-01T10:00:05.000Z"),
          row(5, k[0], "2025-05-15T00:00:01.000Z"),
        ],
        total: 4,
        contactTotal: 4,
        page: 1,
        pageCount: 1,
      })
      expect((await get("message:failed")).json).toEqual({
        data: [row(3, k[3], "2025-07-02T12:00:00.000Z", "user blocked the page")],
        total: 1,
        contactTotal: 1,
        page: 1,
        pageCount: 1,
      })
      expect((await get("message:seen")).json).toMatchObject({ data: [row(2, k[1], "2025-07-02T00:00:00.000Z")], total: 1, contactTotal: 1 })
      expect((await get("flow:clicked")).json).toMatchObject({ data: [row(4, k[2], "2025-07-03T06:00:00.000Z")], total: 1, contactTotal: 1 })
      expect((await get("message:delivered", { perPage: "1", page: "2" })).json).toEqual({
        data: [row(2, k[1], "2025-07-01T23:31:00.000Z")],
        total: 3,
        contactTotal: 3,
        page: 2,
        pageCount: 3,
      })
    })

    it("an empty counter answers an empty page with the counter total", async () => {
      expect((await api.get("/v1/analytics/comment-automation/contacts", { query: { automationId, eventType: "comment:missed" } })).json).toEqual({
        data: [],
        total: 0,
        contactTotal: 0,
        page: 1,
        pageCount: 0,
      })
    })

    it("is 404 notFound for an unknown or foreign automation", async () => {
      const other = await tenantApi("an-comments-404")
      expectError(await other.api.get("/v1/analytics/comment-automation/contacts", { query: { automationId, eventType: "message:sent" } }), 404, "notFound", {
        defined: true,
      })
      expectError(await api.get("/v1/analytics/comment-automation/contacts", { query: { automationId: "1", eventType: "message:sent" } }), 404, "notFound")
    })

    it.each([
      ["no eventType", {}, "eventType"],
      ["an unknown eventType", { eventType: "message:received" }, "eventType"],
      ["a non-numeric automationId", { automationId: "abc", eventType: "message:sent" }, "automationId"],
      ["perPage above 50", { eventType: "message:sent", perPage: "51" }, "perPage"],
    ])("rejects %s (422)", async (_label, query, path) => {
      expectInvalid(await api.get("/v1/analytics/comment-automation/contacts", { query: { automationId, ...query } }), path)
    })
  })

  describe("range input validation", () => {
    it.each([
      ["replies", { ...UTC_RANGE, from: "2025-07-01" }, "from"],
      ["replies", { ...UTC_RANGE, to: "2025-07-03T23:59:59" }, "to"],
      ["user-comments", { ...UTC_RANGE, timezone: "Mars/Olympus" }, "timezone"],
      ["bot-replies", { from: UTC_RANGE.from, to: UTC_RANGE.to }, "timezone"],
      ["errors", { ...UTC_RANGE, automationId: "x1" }, "automationId"],
      ["errors", { ...UTC_RANGE, perPage: 0 }, "perPage"],
    ])("%s rejects %j (422)", async (route, query, path) => {
      expectInvalid(await q(route, query), path)
    })
  })
})
