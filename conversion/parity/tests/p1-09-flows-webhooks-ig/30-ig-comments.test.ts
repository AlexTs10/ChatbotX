// igComments.create / get / update / list / delete / deleteMany: Instagram
// comment automations (`CommentAutomation` rows of type `instagram` or
// `instagramFacebook`). Setup: Instagram accounts through the P1.9 fixture
// extension, flows and Messenger automations through their own public routes.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { createTenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import {
  type Api,
  connectInstagram,
  createFlow,
  DEFAULT_HIDE,
  DEFAULT_OPTIONS,
  expectError,
  expectInvalid,
  expectValidation,
  ext,
  IG_COMMENT_KEYS,
  ISO,
  igCommentBody,
  SNOWFLAKE,
  tenantApi,
  UNKNOWN_ID,
  uniqueDigits,
  uniqueName,
  upstreamCalls,
} from "./helpers-ig.ts"

type IgComment = Record<string, unknown> & { id: string; name: string }

const create = async (api: Api, overrides: Record<string, unknown> = {}): Promise<IgComment> => {
  const response = await api.post<IgComment>("/v1/ig-comments", igCommentBody(overrides))
  expect(response.status, response.text).toBe(201)
  return response.json
}

const NOT_CONNECTED = "The account is not connected to this workspace for this channel"

/** A Messenger comment automation (same table, type messenger) through `fbComments.create`. */
const createMessengerAutomation = async (api: Api): Promise<string> => {
  const { type: _type, ...body } = igCommentBody()
  const response = await api.post<{ id: string }>("/v1/fb-comments", { ...body, type: "messenger" })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

describe("igComments.create", () => {
  it("creates an instagram automation for every account (201) with the full resource", async () => {
    const { api } = await tenantApi("p109-igc-create")
    const since = await fake.cursor()
    const body = igCommentBody({ name: `  ${uniqueName("igc")}  ` })
    const response = await api.post<IgComment>("/v1/ig-comments", body)
    expect(response.status, response.text).toBe(201)
    expect(Object.keys(response.json).sort()).toEqual(IG_COMMENT_KEYS)
    expect(response.json).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
      name: (body.name as string).trim(),
      folderId: null,
      type: "instagram",
      inboxId: null,
      isActive: true,
      startTime: null,
      endTime: null,
      repliesCount: 0,
      sentCount: 0,
      deliveredCount: 0,
      seenCount: 0,
      clickedCount: 0,
      failedCount: 0,
      missedCount: 0,
      moderatedCount: 0,
      post: { type: "all", value: [] },
      privateReply: { type: "text", value: "Thanks for your comment" },
      publicReply: { type: "none", value: null },
      includeKeywords: { type: "all", value: [] },
      excludeKeywords: [],
      // Column default when the request leaves it out.
      excludeKeywordsType: "contain",
      options: DEFAULT_OPTIONS,
      // Instagram cannot detect a GIF: the service writes `hasGif: false`.
      hideComments: { ...DEFAULT_HIDE, hasGif: false },
      replyAfter: { type: "immediately", value: 0 },
    })
    // Not a Live automation: no subscription call.
    expect(await upstreamCalls({ host: "graph.instagram.com", since })).toEqual([])
  })

  it("scopes an instagramFacebook automation to its account and pins unsupported switches off", async () => {
    const { tenant, api } = await tenantApi("p109-igc-create-fb")
    const account = await connectInstagram(tenant, "facebook")
    const created = await create(api, {
      type: "instagramFacebook",
      inboxId: account.inboxId,
      publicReply: { type: "text", value: "Public thanks" },
      includeKeywords: { type: "contain", value: ["price", "Preis"] },
      excludeKeywords: ["spam"],
      excludeKeywordsType: "equal",
      options: { ...DEFAULT_OPTIONS, likeUserComment: true, onlyCommentsWithImage: true },
      hideComments: { ...DEFAULT_HIDE, hasGif: true, hasEmoji: true, hasLink: true, showCommentsAfter: "6h" },
      replyAfter: { type: "minutes", value: 2 },
      post: { type: "postIds", value: ["17890000000000001"] },
    })
    expect(created).toMatchObject({
      type: "instagramFacebook",
      inboxId: account.inboxId,
      post: { type: "postIds", value: ["17890000000000001"] },
      // A public text reply is mirrored into `values`.
      publicReply: { type: "text", value: "Public thanks", values: [{ value: "Public thanks" }] },
      includeKeywords: { type: "contain", value: ["price", "Preis"] },
      excludeKeywords: ["spam"],
      excludeKeywordsType: "equal",
      // Instagram cannot see a comment's photo or GIF: both switches are pinned off.
      options: { ...DEFAULT_OPTIONS, likeUserComment: true, onlyCommentsWithImage: false },
      hideComments: { ...DEFAULT_HIDE, hasGif: false, hasEmoji: true, hasLink: true, showCommentsAfter: "6h" },
      replyAfter: { type: "minutes", value: 2 },
    })
  })

  it("keeps `values` of a public text reply and mirrors its first entry into `value`", async () => {
    const { api } = await tenantApi("p109-igc-values")
    const created = await create(api, {
      publicReply: { type: "text", value: "ignored", values: [{ value: "First" }, { value: "Second" }] },
    })
    expect(created.publicReply).toEqual({
      type: "text",
      value: "First",
      values: [{ value: "First" }, { value: "Second" }],
    })
  })

  it("refuses an inboxId that is not an account of this channel in this workspace (422 validation)", async () => {
    const { tenant, api } = await tenantApi("p109-igc-bad-inbox")
    const login = await connectInstagram(tenant, "instagram")
    // An Instagram Login account on an instagramFacebook automation.
    expectValidation(
      await api.post("/v1/ig-comments", igCommentBody({ type: "instagramFacebook", inboxId: login.inboxId })),
      NOT_CONNECTED,
    )
    // An inbox with no Instagram integration behind it.
    const { inboxId: plainInbox } = await (await ext()).createInbox({
      workspaceId: tenant.workspaceId,
      channel: "instagram",
      name: uniqueName("plain"),
      sourceId: uniqueDigits(),
    })
    expectValidation(await api.post("/v1/ig-comments", igCommentBody({ inboxId: plainInbox })), NOT_CONNECTED)
    // An account of another workspace.
    const other = await createTenant({ label: "p109-igc-bad-inbox-other" })
    const foreign = await connectInstagram(other, "instagram")
    expectValidation(await api.post("/v1/ig-comments", igCommentBody({ inboxId: foreign.inboxId })), NOT_CONNECTED)
    // An unknown id.
    expectValidation(await api.post("/v1/ig-comments", igCommentBody({ inboxId: UNKNOWN_ID })), NOT_CONNECTED)
    // The matching account passes.
    const ok = await create(api, { inboxId: login.inboxId })
    expect(ok.inboxId).toBe(login.inboxId)
    const list = await api.get<{ data: IgComment[] }>("/v1/ig-comments")
    expect(list.json.data.map((item) => item.id)).toEqual([ok.id])
  })

  it("checks a flow reply: own flow passes, unknown or foreign flow is 404 on create (422 on update)", async () => {
    const { api } = await tenantApi("p109-igc-flow")
    const flowId = await createFlow(api)
    const created = await create(api, { privateReply: { type: "flow", value: flowId } })
    expect(created.privateReply).toEqual({ type: "flow", value: flowId })

    // Create locks the referenced flows first; that lock answers 404, not the
    // 422 `validation` of the later existence check (update keeps the 422).
    expectError(
      await api.post("/v1/ig-comments", igCommentBody({ privateReply: { type: "flow", value: UNKNOWN_ID } })),
      404,
      "notFound",
      { defined: false, message: "Flow not found" },
    )
    const { api: otherApi } = await tenantApi("p109-igc-flow-other")
    const foreignFlow = await createFlow(otherApi)
    expectError(
      await api.post("/v1/ig-comments", igCommentBody({ publicReply: { type: "flow", value: foreignFlow } })),
      404,
      "notFound",
      { defined: false, message: "Flow not found" },
    )
    expect((await api.get<{ data: unknown[] }>("/v1/ig-comments")).json.data).toHaveLength(1)
  })

  it("validates the body (422 invalidRequestData)", async () => {
    const { api } = await tenantApi("p109-igc-invalid")
    const post = (overrides: Record<string, unknown>) => api.post("/v1/ig-comments", igCommentBody(overrides))
    expectInvalid(await post({ name: "   " }), "name")
    expectInvalid(await post({ name: "x".repeat(256) }), "name")
    expectInvalid(await post({ type: "messenger" }), "type")
    expectInvalid(await post({ post: { type: "stories", value: [] } }), "post.type")
    expectInvalid(await post({ privateReply: { type: "voice", value: null } }), "privateReply.type")
    expectInvalid(
      await post({
        publicReply: { type: "text", value: "x", values: Array.from({ length: 11 }, (_, i) => ({ value: `m${i}` })) },
      }),
      "publicReply.values",
    )
    expectInvalid(await post({ includeKeywords: { type: "mentions", value: [], mentionCount: 6 } }), "includeKeywords.mentionCount")
    expectInvalid(await post({ excludeKeywordsType: "regex" }), "excludeKeywordsType")
    expectInvalid(await post({ replyAfter: { type: "days", value: 1 } }), "replyAfter.type")
    expectInvalid(await post({ hideComments: { ...DEFAULT_HIDE, showCommentsAfter: "11d" } }), "hideComments.showCommentsAfter")
    expectInvalid(await post({ inboxId: "abc" }), "inboxId")
    const { options: _options, ...noOptions } = igCommentBody()
    expectInvalid(await api.post("/v1/ig-comments", noOptions), "options")
    // Nothing was written.
    const list = await api.get<{ data: unknown[] }>("/v1/ig-comments")
    expect(list.json.data).toEqual([])
  })

  it("AI moderation needs an agent of this workspace (422 validation)", async () => {
    const { api } = await tenantApi("p109-igc-ai-mod")
    const hide = (aiModeration: Record<string, unknown>) => ({ ...DEFAULT_HIDE, aiModeration })
    // Schema refinement: enabled without agent / categories.
    expectInvalid(
      await api.post(
        "/v1/ig-comments",
        igCommentBody({ hideComments: hide({ enabled: true, aiAgentId: null, categories: ["spam"], action: "hide" }) }),
      ),
      "hideComments.aiModeration.aiAgentId",
    )
    expectValidation(
      await api.post(
        "/v1/ig-comments",
        igCommentBody({
          hideComments: hide({ enabled: true, aiAgentId: UNKNOWN_ID, categories: ["spam"], action: "delete" }),
        }),
      ),
      "AI agent not found",
    )
    // Disabled moderation is stored as sent.
    const created = await create(api, {
      hideComments: hide({ enabled: false, aiAgentId: null, categories: [], action: "delete" }),
    })
    expect(created.hideComments).toEqual({
      ...DEFAULT_HIDE,
      hasGif: false,
      aiModeration: { enabled: false, aiAgentId: null, categories: [], action: "delete" },
    })
  })
})

describe("igComments.get", () => {
  it("returns the automation as created; 404 for an unknown, foreign or Messenger id", async () => {
    const { api } = await tenantApi("p109-igc-get")
    const created = await create(api)
    const response = await api.get<IgComment>(`/v1/ig-comments/${created.id}`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual(created)

    const notFound = { message: "Instagram Comment Automation not found" }
    expectError(await api.get(`/v1/ig-comments/${UNKNOWN_ID}`), 404, "notFound", notFound)
    const { api: otherApi } = await tenantApi("p109-igc-get-other")
    const foreign = await create(otherApi)
    expectError(await api.get(`/v1/ig-comments/${foreign.id}`), 404, "notFound", notFound)
    const messengerId = await createMessengerAutomation(api)
    expectError(await api.get(`/v1/ig-comments/${messengerId}`), 404, "notFound", notFound)
    expectInvalid(await api.get("/v1/ig-comments/not-a-number"), "id")
  })
})

describe("igComments.update", () => {
  it("applies a partial update and keeps everything else", async () => {
    const { tenant, api } = await tenantApi("p109-igc-update")
    const account = await connectInstagram(tenant, "instagram")
    const created = await create(api, { inboxId: account.inboxId })
    const newName = uniqueName("renamed")
    const response = await api.patch<IgComment>(`/v1/ig-comments/${created.id}`, {
      name: newName,
      isActive: false,
      startTime: "08:00",
      endTime: "18:30",
      publicReply: { type: "text", value: "Thanks!" },
      // `type` is part of the schema but ignored: an automation never changes channel.
      type: "instagramFacebook",
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      ...created,
      updatedAt: expect.stringMatching(ISO),
      name: newName,
      isActive: false,
      startTime: "08:00",
      endTime: "18:30",
      publicReply: { type: "text", value: "Thanks!", values: [{ value: "Thanks!" }] },
      type: "instagram",
      inboxId: account.inboxId,
    })
    const read = await api.get<IgComment>(`/v1/ig-comments/${created.id}`)
    expect(read.json).toEqual(response.json)
  })

  it("checks a changed inboxId, accepts the stored one, null widens to every account", async () => {
    const { tenant, api } = await tenantApi("p109-igc-update-inbox")
    const account = await connectInstagram(tenant, "instagram")
    const created = await create(api, { inboxId: account.inboxId })
    const other = await connectInstagram(await createTenant({ label: "p109-igc-update-inbox-o" }), "instagram")
    expectValidation(await api.patch(`/v1/ig-comments/${created.id}`, { inboxId: other.inboxId }), NOT_CONNECTED)
    const same = await api.patch<IgComment>(`/v1/ig-comments/${created.id}`, { inboxId: account.inboxId })
    expect(same.status).toBe(200)
    expect(same.json.inboxId).toBe(account.inboxId)
    const widened = await api.patch<IgComment>(`/v1/ig-comments/${created.id}`, { inboxId: null })
    expect(widened.status).toBe(200)
    expect(widened.json.inboxId).toBeNull()
  })

  it("switching to a Live post pins the reply settings Instagram Live cannot do (no subscription call)", async () => {
    const { tenant, api } = await tenantApi("p109-igc-update-live")
    await connectInstagram(tenant, "instagram")
    const created = await create(api, {
      publicReply: { type: "text", value: "Public" },
      options: { ...DEFAULT_OPTIONS, likeUserComment: true, ignoreCommentReplies: true },
      hideComments: { ...DEFAULT_HIDE, all: true, hasLink: true, showCommentsAfter: "1d" },
      replyAfter: { type: "seconds", value: 30 },
    })
    const since = await fake.cursor()
    const response = await api.patch<IgComment>(`/v1/ig-comments/${created.id}`, { post: { type: "live", value: [] } })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({
      post: { type: "live", value: [] },
      publicReply: { type: "none", value: null },
      options: { ...DEFAULT_OPTIONS, likeUserComment: false, ignoreCommentReplies: false },
      hideComments: {
        ...DEFAULT_HIDE,
        all: false,
        hasLink: false,
        hasGif: false,
        hasEmoji: false,
        showCommentsAfter: "none",
      },
      replyAfter: { type: "immediately", value: 0 },
    })
    expect(await upstreamCalls({ host: "graph.instagram.com", since })).toEqual([])
  })

  it("404 for an unknown, foreign or Messenger id; 422 for an invalid field or flow", async () => {
    const { api } = await tenantApi("p109-igc-update-errors")
    const created = await create(api)
    const notFound = { message: "Instagram Comment Automation not found" }
    expectError(await api.patch(`/v1/ig-comments/${UNKNOWN_ID}`, { name: "x" }), 404, "notFound", notFound)
    const { api: otherApi } = await tenantApi("p109-igc-update-errors-o")
    const foreign = await create(otherApi)
    expectError(await api.patch(`/v1/ig-comments/${foreign.id}`, { name: "x" }), 404, "notFound", notFound)
    const messengerId = await createMessengerAutomation(api)
    expectError(await api.patch(`/v1/ig-comments/${messengerId}`, { name: "x" }), 404, "notFound", notFound)
    expectInvalid(await api.patch(`/v1/ig-comments/${created.id}`, { name: "" }), "name")
    expectInvalid(await api.patch(`/v1/ig-comments/${created.id}`, { isActive: "yes" }), "isActive")
    expectValidation(
      await api.patch(`/v1/ig-comments/${created.id}`, { privateReply: { type: "flow", value: UNKNOWN_ID } }),
      "Flow not found",
    )
    // The foreign row was not changed.
    const foreignRead = await otherApi.get<IgComment>(`/v1/ig-comments/${foreign.id}`)
    expect(foreignRead.json.name).toBe(foreign.name)
  })
})

describe("igComments.list", () => {
  it("lists only Instagram automations, with filters and paging", async () => {
    const { tenant, api } = await tenantApi("p109-igc-list")
    const account = await connectInstagram(tenant, "facebook")
    const tag = uniqueName("tag")
    const a = await create(api, { name: `alpha ${tag}` })
    const b = await create(api, { name: `beta ${tag}`, type: "instagramFacebook", inboxId: account.inboxId })
    const c = await create(api, { name: "gamma other" })
    await api.patch(`/v1/ig-comments/${c.id}`, { isActive: false })
    await createMessengerAutomation(api)

    const all = await api.get<{ data: IgComment[]; pageCount: number }>("/v1/ig-comments")
    expect(all.status).toBe(200)
    expect(all.json.pageCount).toBe(1)
    expect(all.json.data.map((item) => item.id).sort()).toEqual([a.id, b.id, c.id].sort())
    expect(all.json.data.find((item) => item.id === a.id)).toEqual(a)

    const byName = await api.get<{ data: IgComment[] }>("/v1/ig-comments", { query: { name: tag.toUpperCase() } })
    expect(byName.json.data.map((item) => item.id).sort()).toEqual([a.id, b.id].sort())

    const inactive = await api.get<{ data: IgComment[] }>("/v1/ig-comments", { query: { isActive: false } })
    expect(inactive.json.data.map((item) => item.id)).toEqual([c.id])

    const byInbox = await api.get<{ data: IgComment[] }>("/v1/ig-comments", { query: { inboxId: account.inboxId } })
    expect(byInbox.json.data.map((item) => item.id)).toEqual([b.id])

    const sortedByName = await api.get<{ data: IgComment[] }>(
      "/v1/ig-comments?sort[0][id]=name&sort[0][desc]=true",
    )
    expect(sortedByName.status, sortedByName.text).toBe(200)
    // All lower case, so the order does not depend on the collation.
    expect(sortedByName.json.data.map((item) => item.id)).toEqual([c.id, b.id, a.id])

    const page = await api.get<{ data: IgComment[]; pageCount: number }>("/v1/ig-comments", {
      query: { perPage: 2, page: 2, name: "a" },
    })
    expect(page.status).toBe(200)
    expect(page.json.pageCount).toBe(2)
    expect(page.json.data).toHaveLength(1)

    expectInvalid(await api.get("/v1/ig-comments", { query: { perPage: 0 } }), "perPage")
    expectInvalid(await api.get("/v1/ig-comments", { query: { page: 0 } }), "page")
  })
})

describe("igComments.delete", () => {
  it("deletes (204); a second delete and foreign / Messenger ids are 404", async () => {
    const { api } = await tenantApi("p109-igc-delete")
    const created = await create(api)
    const response = await api.delete(`/v1/ig-comments/${created.id}`)
    expect(response.status).toBe(204)
    expect(response.text).toBe("")
    expectError(await api.get(`/v1/ig-comments/${created.id}`), 404, "notFound")
    expectError(await api.delete(`/v1/ig-comments/${created.id}`), 404, "notFound", {
      message: "Instagram Comment Automation not found",
    })

    const { api: otherApi } = await tenantApi("p109-igc-delete-o")
    const foreign = await create(otherApi)
    expectError(await api.delete(`/v1/ig-comments/${foreign.id}`), 404, "notFound")
    expect((await otherApi.get(`/v1/ig-comments/${foreign.id}`)).status).toBe(200)

    const messengerId = await createMessengerAutomation(api)
    expectError(await api.delete(`/v1/ig-comments/${messengerId}`), 404, "notFound")
    expect((await api.get(`/v1/fb-comments/${messengerId}`)).status).toBe(200)
  })
})

describe("igComments.deleteMany", () => {
  it("deletes own Instagram automations and ignores foreign, Messenger and unknown ids (204)", async () => {
    const { api } = await tenantApi("p109-igc-bulk")
    const a = await create(api)
    const b = await create(api, { type: "instagramFacebook" })
    const keep = await create(api)
    const messengerId = await createMessengerAutomation(api)
    const { api: otherApi } = await tenantApi("p109-igc-bulk-o")
    const foreign = await create(otherApi)

    const response = await api.post("/v1/ig-comments/bulk-delete", {
      ids: [a.id, b.id, messengerId, foreign.id, UNKNOWN_ID],
    })
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")

    const list = await api.get<{ data: IgComment[] }>("/v1/ig-comments")
    expect(list.json.data.map((item) => item.id)).toEqual([keep.id])
    expect((await api.get(`/v1/fb-comments/${messengerId}`)).status).toBe(200)
    expect((await otherApi.get(`/v1/ig-comments/${foreign.id}`)).status).toBe(200)

    expect((await api.post("/v1/ig-comments/bulk-delete", { ids: [] })).status).toBe(204)
    expectInvalid(await api.post("/v1/ig-comments/bulk-delete", { ids: ["abc"] }), "ids.0")
    expectInvalid(await api.post("/v1/ig-comments/bulk-delete", {}), "ids")
  })
})

describe("igComments token scope", () => {
  it("an `automation`-only token reads and writes Instagram comment automations", async () => {
    const tenant = await createTenant({ label: "p109-igc-scope", scopes: ["automation"] })
    const api = apiClient(tenant.token)
    const created = await create(api)
    expect((await api.get(`/v1/ig-comments/${created.id}`)).status).toBe(200)
  })
})
