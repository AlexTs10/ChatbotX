// fbComments.* — Facebook (Messenger channel) comment automations, and the
// post picker `listPosts` that reads each connected Page's published posts,
// ad-eligible feed posts and reels from Graph (fake upstream).

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import {
  type Api,
  expectError,
  expectInvalid,
  ext,
  GRAPH,
  graphPath,
  keysOf,
  messengerPage,
  qs,
  reEscape,
  sortQuery,
  tenantApi,
  uniqueName,
  upstreamCalls,
} from "./helpers.ts"

const AUTOMATION_KEYS = [
  "clickedCount",
  "createdAt",
  "deliveredCount",
  "endTime",
  "excludeKeywords",
  "excludeKeywordsType",
  "failedCount",
  "folderId",
  "hideComments",
  "id",
  "inboxId",
  "includeKeywords",
  "isActive",
  "missedCount",
  "moderatedCount",
  "name",
  "options",
  "post",
  "privateReply",
  "publicReply",
  "repliesCount",
  "replyAfter",
  "seenCount",
  "sentCount",
  "startTime",
  "type",
  "updatedAt",
]

type FbComment = Record<string, unknown> & { id: string; name: string; inboxId: string | null; folderId: string | null }
type FbCommentList = { data: FbComment[]; pageCount: number }

const OPTIONS = {
  replyToNewContactsOnly: false,
  replyOncePerUserPerPost: true,
  likeUserComment: false,
  replyToUsersWhoCommentedOnOtherPosts: true,
  ignoreCommentReplies: true,
  trackUserTags: false,
}

const HIDE = {
  all: false,
  hasPhoneNumber: true,
  hasImage: false,
  hasVideo: false,
  hasLink: true,
  hasKeywords: true,
  keywords: ["spam"],
  showCommentsAfter: "none",
}

const body = (overrides: Record<string, unknown> = {}) => ({
  name: uniqueName("FB auto"),
  post: { type: "postIds", value: ["111_222"] },
  privateReply: { type: "text", value: "Check your inbox" },
  publicReply: { type: "text", value: "Thanks!" },
  includeKeywords: { type: "contain", value: ["price"] },
  excludeKeywords: ["refund"],
  excludeKeywordsType: "equal",
  options: OPTIONS,
  hideComments: HIDE,
  replyAfter: { type: "seconds", value: 30 },
  ...overrides,
})

const create = async (api: Api, overrides: Record<string, unknown> = {}): Promise<FbComment> => {
  const response = await api.post<FbComment>("/v1/fb-comments", body(overrides))
  expect(response.status, response.text).toBe(201)
  return response.json
}

describe("fbComments.create / get", () => {
  it("creates a Messenger comment automation with zero counters and a normalized public reply", async () => {
    const { tenant, api } = await tenantApi("p108-fc-create")
    const page = await messengerPage(tenant.workspaceId)
    const name = uniqueName("Price replies")
    const created = await create(api, { name, inboxId: page.inboxId, replyAfter: { type: "seconds", value: "30" } })
    expect(keysOf(created)).toEqual(AUTOMATION_KEYS)
    expect(created).toMatchObject({
      name,
      type: "messenger",
      inboxId: page.inboxId,
      folderId: null,
      isActive: true,
      startTime: null,
      endTime: null,
      post: { type: "postIds", value: ["111_222"] },
      privateReply: { type: "text", value: "Check your inbox" },
      // `values` is filled in from `value` on write.
      publicReply: { type: "text", value: "Thanks!", values: [{ value: "Thanks!" }] },
      includeKeywords: { type: "contain", value: ["price"] },
      excludeKeywords: ["refund"],
      excludeKeywordsType: "equal",
      options: OPTIONS,
      hideComments: HIDE,
      replyAfter: { type: "seconds", value: 30 },
      repliesCount: 0,
      sentCount: 0,
      deliveredCount: 0,
      seenCount: 0,
      clickedCount: 0,
      failedCount: 0,
      missedCount: 0,
      moderatedCount: 0,
    })
    expect(created.id).toMatch(/^\d{15,20}$/)

    const got = await api.get<FbComment>(`/v1/fb-comments/${created.id}`)
    expect(got.status).toBe(200)
    expect(got.json).toEqual(created)
  })

  it("defaults: every account (inboxId null), excludeKeywordsType contain; `values` keeps `value` in sync", async () => {
    const { tenant, api } = await tenantApi("p108-fc-defaults")
    const { folderId } = await (await ext()).createFolder({ workspaceId: tenant.workspaceId, name: "FB", folderType: "fbComment" })
    const { excludeKeywordsType: _omit, ...rest } = body()
    const response = await api.post<FbComment>("/v1/fb-comments", {
      ...rest,
      folderId,
      post: { type: "all", value: [] },
      privateReply: { type: "none", value: null },
      publicReply: { type: "text", value: "ignored", values: [{ value: "First" }, { value: "Second" }] },
    })
    expect(response.status, response.text).toBe(201)
    expect(response.json).toMatchObject({
      inboxId: null,
      folderId,
      excludeKeywordsType: "contain",
      privateReply: { type: "none", value: null },
      publicReply: { type: "text", value: "First", values: [{ value: "First" }, { value: "Second" }] },
    })
  })

  it("refuses an account that is not a Page of the workspace (422 validation) and an unknown reply flow (404)", async () => {
    const { tenant, api } = await tenantApi("p108-fc-create-refs")
    const { tenant: other } = await tenantApi("p108-fc-create-refs-other")
    const foreignPage = await messengerPage(other.workspaceId)
    const { inboxId: igInbox } = await (async () => {
      // A webchat inbox of this workspace is not a Messenger account.
      const webchat = await api.post<{ inboxId: string }>("/v1/webchats", {
        name: uniqueName("Chat"),
        brandColor: "#007bff",
        conversationStarters: [],
        persistentMenus: [],
        authorizedDomains: ["example.com"],
      })
      expect(webchat.status, webchat.text).toBe(201)
      return webchat.json
    })()
    expect(tenant.workspaceId).not.toBe(other.workspaceId)
    for (const inboxId of [foreignPage.inboxId, igInbox, "1"]) {
      expectError(await api.post("/v1/fb-comments", body({ inboxId })), 422, "validation", {
        defined: true,
        message: "The account is not connected to this workspace for this channel",
      })
    }
    // An unknown reply flow is refused by the template reference guard (404,
    // not in the create route's error map) before the service's 422 check.
    expectError(await api.post("/v1/fb-comments", body({ privateReply: { type: "flow", value: "1" } })), 404, "notFound", {
      defined: false,
      message: "Flow not found",
    })
    expectError(await api.post("/v1/fb-comments", body({ publicReply: { type: "flow", value: "1" } })), 404, "notFound", {
      defined: false,
      message: "Flow not found",
    })
    expect((await api.get("/v1/fb-comments")).json).toEqual({ data: [], pageCount: 0 })
  })

  it("validates the input (422)", async () => {
    const { api } = await tenantApi("p108-fc-create-invalid")
    const { name: _name, ...noName } = body()
    expectInvalid(await api.post("/v1/fb-comments", noName), "name")
    expectInvalid(await api.post("/v1/fb-comments", body({ name: "  " })), "name")
    expectInvalid(await api.post("/v1/fb-comments", body({ type: "instagram" })), "type")
    expectInvalid(await api.post("/v1/fb-comments", body({ post: { type: "stories", value: [] } })), "post.type")
    expectInvalid(await api.post("/v1/fb-comments", body({ privateReply: { type: "email", value: "x" } })), "privateReply.type")
    expectInvalid(await api.post("/v1/fb-comments", body({ includeKeywords: { type: "mentions", value: [], mentionCount: 6 } })), "includeKeywords.mentionCount")
    expectInvalid(await api.post("/v1/fb-comments", body({ excludeKeywordsType: "regex" })), "excludeKeywordsType")
    expectInvalid(await api.post("/v1/fb-comments", body({ hideComments: { ...HIDE, showCommentsAfter: "2h" } })), "hideComments.showCommentsAfter")
    expectInvalid(await api.post("/v1/fb-comments", body({ replyAfter: { type: "days", value: 1 } })), "replyAfter.type")
    expectInvalid(await api.post("/v1/fb-comments", body({ inboxId: "abc" })), "inboxId")
  })

  it("answers 404 for an unknown or foreign automation and 422 for a non-numeric id", async () => {
    const { api } = await tenantApi("p108-fc-get")
    const { api: other } = await tenantApi("p108-fc-get-other")
    const foreign = await create(other)
    expectError(await api.get("/v1/fb-comments/1"), 404, "notFound", { defined: true, message: "FB Comment Automation not found" })
    expectError(await api.get(`/v1/fb-comments/${foreign.id}`), 404, "notFound")
    expectInvalid(await api.get("/v1/fb-comments/abc"), "id")
  })
})

describe("fbComments.list", () => {
  it("lists every folder by default and filters by name, folder, isActive and account", async () => {
    const { tenant, api } = await tenantApi("p108-fc-list")
    const { api: other } = await tenantApi("p108-fc-list-other")
    const page = await messengerPage(tenant.workspaceId)
    const { folderId } = await (await ext()).createFolder({ workspaceId: tenant.workspaceId, name: "FB", folderType: "fbComment" })
    const a = await create(api, { name: "alpha price" })
    const b = await create(api, { name: "beta ALPHA", folderId, inboxId: page.inboxId })
    const c = await create(api, { name: "gamma" })
    const disabled = await api.patch<FbComment>(`/v1/fb-comments/${c.id}`, { isActive: false })
    expect(disabled.status, disabled.text).toBe(200)
    await create(other, { name: "alpha price" })

    const ids = async (query: string) => {
      const response = await api.get<FbCommentList>(`/v1/fb-comments?${sortQuery([{ id: "name", desc: false }])}${query}`)
      expect(response.status, response.text).toBe(200)
      return response.json
    }
    const all = await ids("")
    expect(all.pageCount).toBe(1)
    expect(all.data.map((row) => row.id)).toEqual([a.id, b.id, c.id])
    expect(all.data[0]).toEqual(a)
    expect((await ids(`&${qs({ name: "Alpha" }).slice(1)}`)).data.map((row) => row.id)).toEqual([a.id, b.id])
    expect((await ids(`&folderId=${folderId}`)).data.map((row) => row.id)).toEqual([b.id])
    expect((await ids("&folderId=0")).data.map((row) => row.id)).toEqual([a.id, c.id])
    expect((await ids("&isActive=false")).data.map((row) => row.id)).toEqual([c.id])
    expect((await ids("&isActive=true")).data.map((row) => row.id)).toEqual([a.id, b.id])
    expect((await ids(`&inboxId=${page.inboxId}`)).data.map((row) => row.id)).toEqual([b.id])
    const page2 = await ids("&perPage=2&page=2")
    expect(page2).toMatchObject({ pageCount: 2, data: [{ id: c.id }] })
    expect((await api.get(`/v1/fb-comments${qs({ name: "zzz" })}`)).json).toEqual({ data: [], pageCount: 0 })
  })
})

describe("fbComments.update", () => {
  it("applies a partial update and returns the full automation", async () => {
    const { tenant, api } = await tenantApi("p108-fc-update")
    const page = await messengerPage(tenant.workspaceId)
    const created = await create(api, { inboxId: page.inboxId })
    const response = await api.patch<FbComment>(`/v1/fb-comments/${created.id}`, {
      name: "Renamed",
      isActive: false,
      startTime: "09:00",
      endTime: "17:30",
      publicReply: { type: "text", value: "New reply" },
      // The stored account again: accepted without a lookup.
      inboxId: page.inboxId,
    })
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(AUTOMATION_KEYS)
    expect(response.json).toEqual({
      ...created,
      name: "Renamed",
      isActive: false,
      startTime: "09:00",
      endTime: "17:30",
      publicReply: { type: "text", value: "New reply", values: [{ value: "New reply" }] },
      updatedAt: expect.any(String),
    })
    expect((await api.get(`/v1/fb-comments/${created.id}`)).json).toEqual(response.json)

    // Back to every account.
    const unscoped = await api.patch<FbComment>(`/v1/fb-comments/${created.id}`, { inboxId: null, startTime: null })
    expect(unscoped.json).toMatchObject({ inboxId: null, startTime: null, endTime: "17:30" })
  })

  it("rejects a foreign account (422), an unknown id (404) and invalid fields (422)", async () => {
    const { api } = await tenantApi("p108-fc-update-errors")
    const { tenant: other } = await tenantApi("p108-fc-update-errors-other")
    const foreignPage = await messengerPage(other.workspaceId)
    const created = await create(api)
    expectError(await api.patch(`/v1/fb-comments/${created.id}`, { inboxId: foreignPage.inboxId }), 422, "validation", {
      message: "The account is not connected to this workspace for this channel",
    })
    expectError(await api.patch("/v1/fb-comments/1", { name: "x" }), 404, "notFound", {
      defined: true,
      message: "FB Comment Automation not found",
    })
    expectInvalid(await api.patch(`/v1/fb-comments/${created.id}`, { isActive: "maybe" }), "isActive")
    expectInvalid(await api.patch(`/v1/fb-comments/${created.id}`, { post: { type: "nope", value: [] } }), "post.type")
  })
})

describe("fbComments.delete / deleteMany", () => {
  it("deletes one automation (204); 404 when it is unknown, foreign or already deleted", async () => {
    const { api } = await tenantApi("p108-fc-delete")
    const { api: other } = await tenantApi("p108-fc-delete-other")
    const mine = await create(api)
    const foreign = await create(other)
    const deleted = await api.delete(`/v1/fb-comments/${mine.id}`)
    expect(deleted.status).toBe(204)
    expect(deleted.text).toBe("")
    expectError(await api.get(`/v1/fb-comments/${mine.id}`), 404, "notFound")
    expectError(await api.delete(`/v1/fb-comments/${mine.id}`), 404, "notFound", { message: "FB Comment Automation not found" })
    expectError(await api.delete(`/v1/fb-comments/${foreign.id}`), 404, "notFound")
    expect((await other.get(`/v1/fb-comments/${foreign.id}`)).status).toBe(200)
  })

  it("deletes several automations (204); ids of another workspace are ignored", async () => {
    const { api } = await tenantApi("p108-fc-bulk")
    const { api: other } = await tenantApi("p108-fc-bulk-other")
    const a = await create(api)
    const b = await create(api)
    const keep = await create(api)
    const foreign = await create(other)
    const response = await api.post("/v1/fb-comments/bulk-delete", { ids: [a.id, b.id, foreign.id, "1"] })
    expect(response.status, response.text).toBe(204)
    expect((await api.get<FbCommentList>("/v1/fb-comments")).json.data.map((row) => row.id)).toEqual([keep.id])
    expect((await other.get(`/v1/fb-comments/${foreign.id}`)).status).toBe(200)
    expect((await api.post("/v1/fb-comments/bulk-delete", { ids: [] })).status).toBe(204)
    expectInvalid(await api.post("/v1/fb-comments/bulk-delete", {}), "ids")
    expectInvalid(await api.post("/v1/fb-comments/bulk-delete", { ids: ["abc"] }), "ids.0")
  })
})

/** Scripts the three post edges of one Page. */
const scriptPosts = async (pageId: string, responses: { published?: unknown[]; feed?: unknown[]; reels?: unknown[] }) => {
  await fake.script(
    {
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${reEscape(pageId)}/posts`) },
      respond: { status: 200, json: { data: responses.published ?? [] } },
    },
    {
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${reEscape(pageId)}/feed`) },
      respond: { status: 200, json: { data: responses.feed ?? [] } },
    },
    {
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${reEscape(pageId)}/video_reels`) },
      respond: { status: 200, json: { data: responses.reels ?? [] } },
    },
  )
}

describe("fbComments.listPosts", () => {
  it("lists published posts, ad-eligible feed posts and reels of every connected Page", async () => {
    const { tenant, api } = await tenantApi("p108-fc-posts")
    const pageA = await messengerPage(tenant.workspaceId, "Page A")
    const pageB = await messengerPage(tenant.workspaceId, "Page B")
    const t = "2026-01-02T03:04:05+0000"
    await scriptPosts(pageA.pageId, {
      published: [{ id: `${pageA.pageId}_1`, message: "Hello", full_picture: "https://cdn.example.com/1.jpg", created_time: t, permalink_url: "https://facebook.com/1" }],
      feed: [
        { id: `${pageA.pageId}_2`, message: "Promo", created_time: t, is_eligible_for_promotion: true },
        { id: `${pageA.pageId}_3`, message: "Not eligible", created_time: t, is_eligible_for_promotion: false },
        { id: `${pageA.pageId}_4`, created_time: t },
      ],
      reels: [
        { id: "9001", post_id: `${pageA.pageId}_5`, description: "Reel", picture: "https://cdn.example.com/r.jpg", created_time: t, permalink_url: "https://facebook.com/r" },
        { id: "9002", created_time: t },
      ],
    })
    await scriptPosts(pageB.pageId, { published: [{ id: `${pageB.pageId}_1`, created_time: t }] })
    const since = await fake.cursor()

    const response = await api.get<{ published: unknown[]; ads: unknown[]; reels: unknown[]; pages: unknown[] }>(
      "/v1/fb-comments/facebook-posts",
    )
    expect(response.status, response.text).toBe(200)
    const byId = <T extends { id: string }>(rows: T[]) => [...rows].sort((x, y) => x.id.localeCompare(y.id))
    expect(byId(response.json.pages as Array<{ id: string }>)).toEqual(
      byId([
        { id: pageA.pageId, name: "Page A" },
        { id: pageB.pageId, name: "Page B" },
      ]),
    )
    expect(byId(response.json.published as Array<{ id: string }>)).toEqual(
      byId([
        {
          id: `${pageA.pageId}_1`,
          message: "Hello",
          full_picture: "https://cdn.example.com/1.jpg",
          created_time: t,
          permalink_url: "https://facebook.com/1",
          pageId: pageA.pageId,
          inboxId: pageA.inboxId,
        },
        { id: `${pageB.pageId}_1`, created_time: t, pageId: pageB.pageId, inboxId: pageB.inboxId },
      ]),
    )
    expect(response.json.ads).toEqual([
      { id: `${pageA.pageId}_2`, message: "Promo", created_time: t, pageId: pageA.pageId, inboxId: pageA.inboxId },
    ])
    expect(response.json.reels).toEqual([
      {
        id: `${pageA.pageId}_5`,
        message: "Reel",
        full_picture: "https://cdn.example.com/r.jpg",
        created_time: t,
        permalink_url: "https://facebook.com/r",
        pageId: pageA.pageId,
        inboxId: pageA.inboxId,
      },
      { id: "9002", created_time: t, pageId: pageA.pageId, inboxId: pageA.inboxId },
    ])

    const callsA = await upstreamCalls({ host: GRAPH, since, path: new RegExp(`^/v[0-9.]+/${pageA.pageId}/`) })
    expect(callsA.map((call) => call.path).sort()).toEqual(
      [`/v23.0/${pageA.pageId}/feed`, `/v23.0/${pageA.pageId}/posts`, `/v23.0/${pageA.pageId}/video_reels`].sort(),
    )
    for (const call of callsA) {
      expect(call.headers.authorization).toBe(`Bearer ${pageA.accessToken}`)
      expect(call.query.limit).toBe("100")
    }
    expect(callsA.find((call) => call.path.endsWith("/posts"))?.query.fields).toBe("id,message,full_picture,created_time,permalink_url")
    expect(callsA.find((call) => call.path.endsWith("/feed"))?.query.fields).toBe(
      "id,message,full_picture,created_time,permalink_url,is_eligible_for_promotion",
    )
    expect(callsA.find((call) => call.path.endsWith("/video_reels"))?.query.fields).toBe(
      "id,post_id,description,picture,created_time,permalink_url",
    )
  })

  it("narrows to one account with inboxId and drops a post type whose Graph call fails", async () => {
    const { tenant, api } = await tenantApi("p108-fc-posts-filter")
    const pageA = await messengerPage(tenant.workspaceId, "Only A")
    const pageB = await messengerPage(tenant.workspaceId, "Not B")
    const t = "2026-01-02T03:04:05+0000"
    await scriptPosts(pageA.pageId, { published: [{ id: `${pageA.pageId}_1`, created_time: t }], reels: [] })
    await fake.script({
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${reEscape(pageA.pageId)}/feed`) },
      respond: { status: 400, json: { error: { message: "(#10) Permission denied", type: "OAuthException", code: 10 } } },
    })
    const since = await fake.cursor()
    const response = await api.get(`/v1/fb-comments/facebook-posts${qs({ inboxId: pageA.inboxId })}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      published: [{ id: `${pageA.pageId}_1`, created_time: t, pageId: pageA.pageId, inboxId: pageA.inboxId }],
      ads: [],
      reels: [],
      pages: [{ id: pageA.pageId, name: "Only A" }],
    })
    expect(await upstreamCalls({ host: GRAPH, since, path: new RegExp(`/${pageB.pageId}/`) })).toHaveLength(0)

    // No Page (or an inboxId of no Page): no Graph call, empty lists.
    const none = await api.get(`/v1/fb-comments/facebook-posts${qs({ inboxId: "1" })}`)
    expect(none.json).toEqual({ published: [], ads: [], reels: [], pages: [] })
    const { api: empty } = await tenantApi("p108-fc-posts-empty")
    expect((await empty.get("/v1/fb-comments/facebook-posts")).json).toEqual({ published: [], ads: [], reels: [], pages: [] })
    expectInvalid(await api.get(`/v1/fb-comments/facebook-posts${qs({ inboxId: "abc" })}`), "inboxId")
  })
})
