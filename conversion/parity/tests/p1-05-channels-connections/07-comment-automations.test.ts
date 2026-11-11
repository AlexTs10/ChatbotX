// commentAutomations.* — accounts per channel type, missed-comment runs
// (scan the post's comments on Graph, queue a replay per unhandled comment,
// the worker answers through the live comment path) and the moderation log.
// Channel, automation and moderation-log rows are fixture rows (they come
// from OAuth connects, the per-channel automation routes and the worker).

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { fake } from "../../src/fake.ts"
import type { CreateCommentAutomationInput } from "../../src/fixtures/ext/p1-05.ts"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  ext,
  expectError,
  expectInvalid,
  graphPath,
  qs,
  reEscape,
  script,
  tenantApi,
  uniqueDigits,
  uniqueName,
  upstreamCalls,
} from "./helpers.ts"

const GRAPH = "graph.facebook.com"

type Accounts = { accounts: Array<{ inboxId: string; name: string }> }
type RunResult = { status: string; reason?: string; scanned?: number; skipped?: number; queued?: number; failed?: number }

const automation = async (input: Omit<CreateCommentAutomationInput, "name"> & { name?: string }) =>
  (await (await ext()).createCommentAutomation({ name: uniqueName("auto"), ...input })).automationId

const messengerPage = async (workspaceId: string) => {
  const pageId = uniqueDigits()
  const channel = await (await ext()).createMessengerChannel({ workspaceId, pageId, name: `Page ${pageId}` })
  return { pageId, ...channel }
}

/** "HH:mm" (UTC) `hours` from now. */
const hhmm = (hours: number) => new Date(Date.now() + hours * 3_600_000).toISOString().slice(11, 16)

describe("commentAutomations.listAccounts", () => {
  it("lists the connected accounts of one automation channel type", async () => {
    const { tenant, api } = await tenantApi("p105-ca-accounts")
    const fixtures = await ext()
    const pageA = await messengerPage(tenant.workspaceId)
    const pageB = await messengerPage(tenant.workspaceId)
    const igLogin = await fixtures.createInstagramChannel({
      workspaceId: tenant.workspaceId,
      igId: uniqueDigits(),
      pageId: uniqueDigits(),
      name: "IG login",
      username: `ig_${uniqueId()}`,
      type: "instagram",
    })
    const igFacebook = await fixtures.createInstagramChannel({
      workspaceId: tenant.workspaceId,
      igId: uniqueDigits(),
      pageId: uniqueDigits(),
      name: "IG via Facebook",
      username: `igfb_${uniqueId()}`,
      type: "facebook",
    })
    // Another workspace's page is not listed.
    const { tenant: other } = await tenantApi("p105-ca-accounts-other")
    await messengerPage(other.workspaceId)

    const list = async (type: string) => {
      const response = await api.get<Accounts>(`/v1/comment-automations/accounts${qs({ type })}`)
      expect(response.status).toBe(200)
      return [...response.json.accounts].sort((a, b) => a.name.localeCompare(b.name))
    }
    expect(await list("messenger")).toEqual(
      [
        { inboxId: pageA.inboxId, name: `Page ${pageA.pageId}` },
        { inboxId: pageB.inboxId, name: `Page ${pageB.pageId}` },
      ].sort((a, b) => a.name.localeCompare(b.name)),
    )
    expect(await list("instagram")).toEqual([{ inboxId: igLogin.inboxId, name: "IG login" }])
    expect(await list("instagramFacebook")).toEqual([{ inboxId: igFacebook.inboxId, name: "IG via Facebook" }])
    expect(await list("threads")).toEqual([])
    expect(await list("tiktok")).toEqual([])
  })

  it("requires a valid type (422)", async () => {
    const { api } = await tenantApi("p105-ca-accounts-bad")
    expectInvalid(await api.get("/v1/comment-automations/accounts"), "type")
    expectInvalid(await api.get(`/v1/comment-automations/accounts${qs({ type: "whatsapp" })}`), "type")
  })
})

describe("commentAutomations.getMissedCommentRun", () => {
  it("reports no run in progress; 404 for an unknown or foreign automation; 422 for a non-numeric id", async () => {
    const { tenant, api } = await tenantApi("p105-ca-run-get")
    const { tenant: other } = await tenantApi("p105-ca-run-get-other")
    const id = await automation({ workspaceId: tenant.workspaceId, type: "messenger" })
    const foreign = await automation({ workspaceId: other.workspaceId, type: "messenger" })
    const response = await api.get(`/v1/comment-automations/${id}/missed-comment-runs`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ inProgress: false })
    expectError(await api.get("/v1/comment-automations/1/missed-comment-runs"), 404, "notFound", {
      defined: true,
      message: "Comment Automation not found",
    })
    expectError(await api.get(`/v1/comment-automations/${foreign}/missed-comment-runs`), 404, "notFound")
    expectInvalid(await api.get("/v1/comment-automations/abc/missed-comment-runs"), "id")
  })
})

describe("commentAutomations.processMissedComments", () => {
  it("refuses an automation that cannot run, with the reason", async () => {
    const { tenant, api } = await tenantApi("p105-ca-run-reasons")
    const ws = tenant.workspaceId
    const run = async (id: string) => {
      const response = await api.post<RunResult>(`/v1/comment-automations/${id}/missed-comment-runs`, {})
      expect(response.status).toBe(200)
      return response.json
    }
    expect(await run(await automation({ workspaceId: ws, type: "messenger" }))).toEqual({
      status: "failed",
      reason: "notSinglePost",
    })
    expect(
      await run(await automation({ workspaceId: ws, type: "messenger", post: { type: "postIds", value: ["1_2", "1_3"] } })),
    ).toEqual({ status: "failed", reason: "notSinglePost" })
    expect(
      await run(
        await automation({ workspaceId: ws, type: "messenger", isActive: false, post: { type: "postIds", value: ["1_2"] } }),
      ),
    ).toEqual({ status: "failed", reason: "inactive" })
    expect(
      await run(
        await automation({
          workspaceId: ws,
          type: "messenger",
          post: { type: "postIds", value: ["1_2"] },
          startTime: hhmm(2),
          endTime: hhmm(3),
        }),
      ),
    ).toEqual({ status: "failed", reason: "outsideSchedule" })
    // No Messenger page of that id is connected in the workspace.
    expect(
      await run(
        await automation({ workspaceId: ws, type: "messenger", post: { type: "postIds", value: [`${uniqueDigits()}_9`] } }),
      ),
    ).toEqual({ status: "failed", reason: "integrationNotFound" })
  })

  it("404s an unknown automation", async () => {
    const { api } = await tenantApi("p105-ca-run-404")
    expectError(await api.post("/v1/comment-automations/1/missed-comment-runs", {}), 404, "notFound", {
      message: "Comment automation not found",
    })
  })

  it("reports fetchFailed when Graph refuses the comment list", async () => {
    const { tenant, api } = await tenantApi("p105-ca-run-fetchfail")
    const page = await messengerPage(tenant.workspaceId)
    const postId = `${page.pageId}_${uniqueDigits()}`
    const id = await automation({ workspaceId: tenant.workspaceId, type: "messenger", post: { type: "postIds", value: [postId] } })
    await script({
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${reEscape(postId)}/comments`) },
      respond: { status: 400, json: { error: { message: "Unsupported get request.", type: "GraphMethodException", code: 100 } } },
    })
    const response = await api.post<RunResult>(`/v1/comment-automations/${id}/missed-comment-runs`, {})
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ status: "failed", reason: "fetchFailed" })
    expect((await api.get(`/v1/comment-automations/${id}/missed-comment-runs`)).json).toEqual({ inProgress: false })
  })

  it("scans the post on Graph and reports an empty run", async () => {
    const { tenant, api } = await tenantApi("p105-ca-run-empty")
    const page = await messengerPage(tenant.workspaceId)
    const postId = `${page.pageId}_${uniqueDigits()}`
    const id = await automation({ workspaceId: tenant.workspaceId, type: "messenger", post: { type: "postIds", value: [postId] } })
    const since = await fake.cursor()
    await script({
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${reEscape(postId)}/comments`) },
      respond: { status: 200, json: { data: [] } },
    })
    const response = await api.post<RunResult>(`/v1/comment-automations/${id}/missed-comment-runs`, {})
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ status: "done", scanned: 0, skipped: 0, queued: 0, failed: 0 })
    const calls = await upstreamCalls({ host: GRAPH, since, path: new RegExp(`/${reEscape(postId)}/comments$`) })
    expect(calls).toHaveLength(1)
    expect(calls[0]?.headers.authorization).toBe(`Bearer page-token-${page.pageId}`)
    expect(calls[0]?.query).toMatchObject({ order: "reverse_chronological", filter: "stream", limit: "100" })
    expect((await api.get(`/v1/comment-automations/${id}/missed-comment-runs`)).json).toEqual({ inProgress: false })
  })

  it("queues unhandled recent comments; the worker posts the public reply on Graph", async () => {
    const { tenant, api } = await tenantApi("p105-ca-run-reply")
    const page = await messengerPage(tenant.workspaceId)
    const storyId = uniqueDigits()
    const postId = `${page.pageId}_${storyId}`
    const fromId = uniqueDigits()
    const commentId = `${storyId}_${uniqueDigits()}`
    const replyText = `Thanks ${uniqueId()}`
    const id = await automation({
      workspaceId: tenant.workspaceId,
      type: "messenger",
      post: { type: "postIds", value: [postId] },
      publicReply: { type: "text", value: replyText },
    })
    // Every Graph call about this page, post, comment or commenter is answered.
    for (const fragment of [page.pageId, storyId, fromId]) {
      await script({
        match: { host: GRAPH, pathRegex: `^/v[0-9.]+/[^?]*${reEscape(fragment)}` },
        respond: { status: 200, json: { id: `${fragment}_${uniqueDigits()}`, success: true } },
      })
    }
    await script({
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${reEscape(postId)}/comments`) },
      respond: {
        status: 200,
        json: {
          data: [
            {
              id: commentId,
              message: "How much is it?",
              created_time: new Date(Date.now() - 60_000).toISOString().replace(/\.\d{3}Z$/, "+0000"),
              from: { id: fromId, name: "Ana Lima" },
            },
            // The page's own comment and a comment older than 7 days are not replayed.
            {
              id: `${storyId}_${uniqueDigits()}`,
              message: "Page reply",
              created_time: new Date(Date.now() - 120_000).toISOString().replace(/\.\d{3}Z$/, "+0000"),
              from: { id: page.pageId, name: "Page" },
            },
            {
              id: `${storyId}_${uniqueDigits()}`,
              message: "Old",
              created_time: new Date(Date.now() - 8 * 86_400_000).toISOString().replace(/\.\d{3}Z$/, "+0000"),
              from: { id: uniqueDigits(), name: "Old commenter" },
            },
          ],
        },
      },
    })
    const since = await fake.cursor()
    const response = await api.post<RunResult>(`/v1/comment-automations/${id}/missed-comment-runs`, {})
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ status: "done", scanned: 1, skipped: 0, queued: 1, failed: 0 })

    const replies = await eventually(
      async () => {
        const calls = await upstreamCalls({
          host: GRAPH,
          since,
          method: "POST",
          path: new RegExp(`^/v[0-9.]+/${reEscape(commentId)}/comments$`),
        })
        expect(calls.length).toBeGreaterThanOrEqual(1)
        return calls
      },
      { timeoutMs: 30_000, intervalMs: 500, description: "public reply posted on Graph" },
    )
    expect(replies[0]?.bodyText).toContain(replyText)

    await eventually(
      async () => {
        expect((await api.get(`/v1/comment-automations/${id}/missed-comment-runs`)).json).toEqual({ inProgress: false })
      },
      { timeoutMs: 30_000, intervalMs: 500, description: "missed comment run settles" },
    )
    // The comment now has an event row: a second run skips it.
    const again = await api.post<RunResult>(`/v1/comment-automations/${id}/missed-comment-runs`, {})
    expect(again.json).toEqual({ status: "done", scanned: 1, skipped: 1, queued: 0, failed: 0 })
  })
})

describe("commentAutomations moderation log", () => {
  const seed = async () => {
    const { tenant, api } = await tenantApi("p105-ca-modlog")
    const fixtures = await ext()
    const page = await messengerPage(tenant.workspaceId)
    const id = await automation({ workspaceId: tenant.workspaceId, type: "messenger" })
    const contact = await fixtures.createContact({
      workspaceId: tenant.workspaceId,
      inboxId: page.inboxId,
      channel: "messenger",
      sourceId: `psid-${uniqueId()}`,
      firstName: "Mia",
      lastName: "Moderated",
    })
    const postId = `${page.pageId}_${uniqueDigits()}`
    const rows = [
      {
        commentId: "c-old",
        occurredAt: "2026-10-01T10:00:00.000Z",
        source: "rule" as const,
        rule: "keyword" as const,
        matchedKeyword: "spam",
        action: "hide" as const,
        status: "done" as const,
        commentText: "buy spam now",
        contactId: contact.contactId,
        contactInboxId: contact.contactInboxId,
      },
      {
        commentId: "c-mid",
        occurredAt: "2026-10-02T10:00:00.000Z",
        source: "ai" as const,
        categories: ["offensive" as const, "harassment" as const],
        reason: "Insults another user.",
        action: "delete" as const,
        status: "failed" as const,
        errorDetail: "(#200) Permissions error",
        commentText: "you are ...",
      },
      {
        commentId: "c-new",
        occurredAt: "2026-10-03T10:00:00.000Z",
        source: "rule" as const,
        rule: "link" as const,
        action: "hide" as const,
        commentText: null,
      },
    ]
    const { ids } = await fixtures.insertModerationLogs(
      rows.map((row) => ({ workspaceId: tenant.workspaceId, automationId: id, postId, ...row })),
    )
    return { api, tenant, id, postId, contact, ids }
  }

  it("lists moderated comments newest first, with the commenter as ids only", async () => {
    const { api, id, postId, contact, ids } = await seed()
    const response = await api.get<{ data: Array<Record<string, unknown>>; total: number; page: number; pageCount: number }>(
      `/v1/comment-automations/${id}/moderation-logs`,
    )
    expect(response.status).toBe(200)
    const normalizer = new Normalizer()
    expect(
      normalizer.normalize(response.json, {
        replace: {
          [ids[0] as string]: "<log-old>",
          [ids[1] as string]: "<log-mid>",
          [ids[2] as string]: "<log-new>",
          [contact.contactId]: "<contact>",
          [contact.contactInboxId]: "<contactInbox>",
          [contact.conversationId]: "<conversation>",
          [postId]: "<post>",
        },
        checkTimestamps: false,
      }),
    ).toEqual({
      data: [
        {
          action: "hide",
          categories: [],
          channel: null,
          commentId: "c-new",
          commentText: null,
          contactId: null,
          contactInboxId: null,
          conversationId: null,
          errorDetail: null,
          id: "<log-new>",
          matchedKeyword: null,
          occurredAt: "<ts>",
          postId: "<post>",
          reason: null,
          rule: "link",
          source: "rule",
          sourceId: null,
          status: "pending",
        },
        {
          action: "delete",
          categories: ["offensive", "harassment"],
          channel: null,
          commentId: "c-mid",
          commentText: "you are ...",
          contactId: null,
          contactInboxId: null,
          conversationId: null,
          errorDetail: "(#200) Permissions error",
          id: "<log-mid>",
          matchedKeyword: null,
          occurredAt: "<ts>",
          postId: "<post>",
          reason: "Insults another user.",
          rule: null,
          source: "ai",
          sourceId: null,
          status: "failed",
        },
        {
          action: "hide",
          categories: [],
          channel: "messenger",
          commentId: "c-old",
          commentText: "buy spam now",
          contactId: "<contact>",
          contactInboxId: "<contactInbox>",
          conversationId: "<conversation>",
          errorDetail: null,
          id: "<log-old>",
          matchedKeyword: "spam",
          occurredAt: "<ts>",
          postId: "<post>",
          reason: null,
          rule: "keyword",
          source: "rule",
          sourceId: expect.stringMatching(/^psid-/),
          status: "done",
        },
      ],
      page: 1,
      pageCount: 1,
      total: 3,
    })
    expect(response.json.data.map((row) => row.occurredAt)).toEqual([
      "2026-10-03T10:00:00.000Z",
      "2026-10-02T10:00:00.000Z",
      "2026-10-01T10:00:00.000Z",
    ])
    expect(response.json.data.every((row) => !("fullName" in row) && !("avatar" in row))).toBe(true)
  })

  it("filters by time range, action and source, and pages", async () => {
    const { api, id } = await seed()
    const commentIds = async (query: Record<string, string | number>) => {
      const response = await api.get<{ data: Array<{ commentId: string }>; total: number; pageCount: number }>(
        `/v1/comment-automations/${id}/moderation-logs${qs(query)}`,
      )
      expect(response.status).toBe(200)
      return { ids: response.json.data.map((row) => row.commentId), total: response.json.total, pageCount: response.json.pageCount }
    }
    expect((await commentIds({ action: "hide" })).ids).toEqual(["c-new", "c-old"])
    expect((await commentIds({ action: "delete" })).ids).toEqual(["c-mid"])
    expect((await commentIds({ source: "ai" })).ids).toEqual(["c-mid"])
    expect((await commentIds({ source: "rule", action: "hide" })).ids).toEqual(["c-new", "c-old"])
    // from/to are inclusive and accept an offset.
    expect((await commentIds({ from: "2026-10-02T10:00:00.000Z" })).ids).toEqual(["c-new", "c-mid"])
    expect((await commentIds({ to: "2026-10-02T17:00:00+07:00" })).ids).toEqual(["c-mid", "c-old"])
    expect(await commentIds({ perPage: 2 })).toEqual({ ids: ["c-new", "c-mid"], total: 3, pageCount: 2 })
    expect(await commentIds({ perPage: 2, page: 2 })).toEqual({ ids: ["c-old"], total: 3, pageCount: 2 })
  })

  it("summarizes by action, source and outcome with the same filters", async () => {
    const { api, id } = await seed()
    const summary = await api.get(`/v1/comment-automations/${id}/moderation-summary`)
    expect(summary.status).toBe(200)
    expect(summary.json).toEqual({ total: 3, hidden: 2, deleted: 1, byRule: 2, byAI: 1, failed: 1, pending: 1 })
    const ruleOnly = await api.get(`/v1/comment-automations/${id}/moderation-summary${qs({ source: "rule" })}`)
    expect(ruleOnly.json).toEqual({ total: 2, hidden: 2, deleted: 0, byRule: 2, byAI: 0, failed: 0, pending: 1 })
    const ranged = await api.get(
      `/v1/comment-automations/${id}/moderation-summary${qs({ from: "2026-10-02T00:00:00Z", to: "2026-10-02T23:59:59Z" })}`,
    )
    expect(ranged.json).toEqual({ total: 1, hidden: 0, deleted: 1, byRule: 0, byAI: 1, failed: 1, pending: 0 })
  })

  it("answers an automation without moderated comments with zeroes", async () => {
    const { tenant, api } = await tenantApi("p105-ca-modlog-empty")
    const id = await automation({ workspaceId: tenant.workspaceId, type: "instagram" })
    expect((await api.get(`/v1/comment-automations/${id}/moderation-logs`)).json).toEqual({
      data: [],
      total: 0,
      page: 1,
      pageCount: 0,
    })
    expect((await api.get(`/v1/comment-automations/${id}/moderation-summary`)).json).toEqual({
      total: 0,
      hidden: 0,
      deleted: 0,
      byRule: 0,
      byAI: 0,
      failed: 0,
      pending: 0,
    })
  })

  it("404s an unknown or foreign automation and validates the filters (422)", async () => {
    const { api, id } = await seed()
    const { api: other } = await tenantApi("p105-ca-modlog-other")
    expectError(await other.get(`/v1/comment-automations/${id}/moderation-logs`), 404, "notFound", {
      message: "Comment Automation not found",
    })
    expectError(await other.get(`/v1/comment-automations/${id}/moderation-summary`), 404, "notFound")
    expectError(await api.get("/v1/comment-automations/1/moderation-logs"), 404, "notFound")
    expectInvalid(await api.get(`/v1/comment-automations/${id}/moderation-logs${qs({ from: "yesterday" })}`), "from")
    expectInvalid(await api.get(`/v1/comment-automations/${id}/moderation-logs${qs({ to: "2026-10-02" })}`), "to")
    expectInvalid(await api.get(`/v1/comment-automations/${id}/moderation-logs${qs({ action: "ban" })}`), "action")
    expectInvalid(await api.get(`/v1/comment-automations/${id}/moderation-summary${qs({ source: "human" })}`), "source")
    expectInvalid(await api.get(`/v1/comment-automations/${id}/moderation-logs${qs({ perPage: 51 })}`), "perPage")
    expectInvalid(await api.get("/v1/comment-automations/abc/moderation-summary"), "id")
  })
})
