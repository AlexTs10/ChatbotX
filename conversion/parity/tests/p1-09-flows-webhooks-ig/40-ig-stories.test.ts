// igStories.*: Instagram story reply / story mention automations
// (`IgStoryAutomation`), and igStories.listStories (Graph API `stories` edge
// of the connected accounts, scripted on the fake upstream).

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { createTenant } from "../../src/fixtures/index.ts"
import {
  type Api,
  connectInstagram,
  expectError,
  expectInvalid,
  expectValidation,
  ext,
  graphPath,
  IG_STORY_KEYS,
  ISO,
  SNOWFLAKE,
  tenantApi,
  UNKNOWN_ID,
  uniqueDigits,
  uniqueName,
  upstreamCalls,
} from "./helpers-ig.ts"

type IgStory = Record<string, unknown> & { id: string; name: string }

const storyBody = (overrides: Record<string, unknown> = {}): Record<string, unknown> => ({
  name: uniqueName("igs"),
  type: "instagram",
  reply: { type: "text", value: "Thanks for the reply" },
  ...overrides,
})

const create = async (api: Api, overrides: Record<string, unknown> = {}): Promise<IgStory> => {
  const response = await api.post<IgStory>("/v1/ig-stories", storyBody(overrides))
  expect(response.status, response.text).toBe(201)
  return response.json
}

const NOT_CONNECTED = "The account is not connected to this workspace for this channel"
const NOT_FOUND = { message: "Instagram Story Automation not found" }

describe("igStories.create", () => {
  it("a minimal body is a storyReply automation on all stories and all replies (201)", async () => {
    const { api } = await tenantApi("p109-igs-create")
    const body = storyBody()
    const response = await api.post<IgStory>("/v1/ig-stories", body)
    expect(response.status, response.text).toBe(201)
    expect(Object.keys(response.json).sort()).toEqual(IG_STORY_KEYS)
    expect(response.json).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
      name: body.name,
      folderId: null,
      type: "instagram",
      trigger: "storyReply",
      inboxId: null,
      isActive: true,
      startTime: null,
      endTime: null,
      repliesCount: 0,
      story: { type: "all", value: [] },
      // Not normalized: a story reply keeps the reply exactly as sent.
      reply: { type: "text", value: "Thanks for the reply" },
      includeKeywords: { type: "all", value: [] },
    })
  })

  it("stores story ids and keywords for storyReply, scoped to an account", async () => {
    const { tenant, api } = await tenantApi("p109-igs-create-scoped")
    const account = await connectInstagram(tenant, "facebook")
    const created = await create(api, {
      type: "instagramFacebook",
      trigger: "storyReply",
      inboxId: account.inboxId,
      story: { type: "storyIds", value: ["s-1", "s-2"] },
      includeKeywords: { type: "equal", value: ["yes"] },
      reply: { type: "text", value: "Hi", values: [{ value: "Hi" }, { value: "Again" }] },
    })
    expect(created).toMatchObject({
      type: "instagramFacebook",
      trigger: "storyReply",
      inboxId: account.inboxId,
      story: { type: "storyIds", value: ["s-1", "s-2"] },
      includeKeywords: { type: "equal", value: ["yes"] },
      reply: { type: "text", value: "Hi", values: [{ value: "Hi" }, { value: "Again" }] },
    })
  })

  it("storyMention ignores story and keyword filters (always `all`)", async () => {
    const { api } = await tenantApi("p109-igs-mention")
    const created = await create(api, {
      trigger: "storyMention",
      story: { type: "storyIds", value: ["s-1"] },
      includeKeywords: { type: "contain", value: ["x"] },
    })
    expect(created).toMatchObject({
      trigger: "storyMention",
      story: { type: "all", value: [] },
      includeKeywords: { type: "all", value: [] },
    })
  })

  it("does not check a flow reply (an unknown flow id is stored)", async () => {
    const { api } = await tenantApi("p109-igs-flow")
    const created = await create(api, { reply: { type: "flow", value: UNKNOWN_ID } })
    expect(created.reply).toEqual({ type: "flow", value: UNKNOWN_ID })
  })

  it("refuses an inboxId of another channel type, another workspace or no integration (422 validation)", async () => {
    const { tenant, api } = await tenantApi("p109-igs-bad-inbox")
    const login = await connectInstagram(tenant, "instagram")
    expectValidation(
      await api.post("/v1/ig-stories", storyBody({ type: "instagramFacebook", inboxId: login.inboxId })),
      NOT_CONNECTED,
    )
    const foreign = await connectInstagram(await createTenant({ label: "p109-igs-bad-inbox-o" }), "instagram")
    expectValidation(await api.post("/v1/ig-stories", storyBody({ inboxId: foreign.inboxId })), NOT_CONNECTED)
    const { inboxId } = await (await ext()).createInbox({
      workspaceId: tenant.workspaceId,
      channel: "instagram",
      name: uniqueName("plain"),
      sourceId: uniqueDigits(),
    })
    expectValidation(await api.post("/v1/ig-stories", storyBody({ inboxId })), NOT_CONNECTED)
    const list = await api.get<{ data: IgStory[] }>("/v1/ig-stories")
    expect(list.json.data).toEqual([])
  })

  it("validates the body (422 invalidRequestData)", async () => {
    const { api } = await tenantApi("p109-igs-invalid")
    const post = (overrides: Record<string, unknown>) => api.post("/v1/ig-stories", storyBody(overrides))
    expectInvalid(await post({ name: "" }), "name")
    expectInvalid(await post({ type: "messenger" }), "type")
    expectInvalid(await post({ trigger: "storyLike" }), "trigger")
    expectInvalid(await post({ story: { type: "postIds", value: [] } }), "story.type")
    expectInvalid(await post({ reply: { type: "voice", value: null } }), "reply.type")
    expectInvalid(await post({ includeKeywords: { type: "regex", value: [] } }), "includeKeywords.type")
    expectInvalid(await post({ inboxId: "abc" }), "inboxId")
    const { reply: _reply, ...noReply } = storyBody()
    expectInvalid(await api.post("/v1/ig-stories", noReply), "reply")
  })
})

describe("igStories.get / update", () => {
  it("get returns the automation; 404 for unknown and foreign ids", async () => {
    const { api } = await tenantApi("p109-igs-get")
    const created = await create(api)
    const response = await api.get<IgStory>(`/v1/ig-stories/${created.id}`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual(created)
    expectError(await api.get(`/v1/ig-stories/${UNKNOWN_ID}`), 404, "notFound", NOT_FOUND)
    const { api: otherApi } = await tenantApi("p109-igs-get-o")
    const foreign = await create(otherApi)
    expectError(await api.get(`/v1/ig-stories/${foreign.id}`), 404, "notFound", NOT_FOUND)
  })

  it("update is partial; `trigger` and `type` never change", async () => {
    const { tenant, api } = await tenantApi("p109-igs-update")
    const account = await connectInstagram(tenant, "instagram")
    const created = await create(api)
    const name = uniqueName("renamed")
    const response = await api.patch<IgStory>(`/v1/ig-stories/${created.id}`, {
      name,
      isActive: false,
      startTime: "09:00",
      endTime: null,
      inboxId: account.inboxId,
      story: { type: "storyIds", value: ["s-9"] },
      trigger: "storyMention",
      type: "instagramFacebook",
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      ...created,
      updatedAt: expect.stringMatching(ISO),
      name,
      isActive: false,
      startTime: "09:00",
      endTime: null,
      inboxId: account.inboxId,
      story: { type: "storyIds", value: ["s-9"] },
      trigger: "storyReply",
      type: "instagram",
    })
    expect((await api.get<IgStory>(`/v1/ig-stories/${created.id}`)).json).toEqual(response.json)
  })

  it("update of a storyMention automation keeps story and keywords at `all`", async () => {
    const { api } = await tenantApi("p109-igs-update-mention")
    const created = await create(api, { trigger: "storyMention" })
    const response = await api.patch<IgStory>(`/v1/ig-stories/${created.id}`, {
      story: { type: "storyIds", value: ["s-1"] },
      includeKeywords: { type: "contain", value: ["x"] },
      reply: { type: "none", value: null },
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({
      trigger: "storyMention",
      story: { type: "all", value: [] },
      includeKeywords: { type: "all", value: [] },
      reply: { type: "none", value: null },
    })
  })

  it("update errors: 404 unknown / foreign, 422 bad field or account", async () => {
    const { api } = await tenantApi("p109-igs-update-errors")
    const created = await create(api)
    expectError(await api.patch(`/v1/ig-stories/${UNKNOWN_ID}`, { name: "x" }), 404, "notFound", NOT_FOUND)
    const { api: otherApi } = await tenantApi("p109-igs-update-errors-o")
    const foreign = await create(otherApi)
    expectError(await api.patch(`/v1/ig-stories/${foreign.id}`, { name: "x" }), 404, "notFound", NOT_FOUND)
    expectInvalid(await api.patch(`/v1/ig-stories/${created.id}`, { name: "" }), "name")
    expectValidation(await api.patch(`/v1/ig-stories/${created.id}`, { inboxId: UNKNOWN_ID }), NOT_CONNECTED)
  })
})

describe("igStories.list", () => {
  it("filters by trigger, name, isActive and inboxId; pages", async () => {
    const { tenant, api } = await tenantApi("p109-igs-list")
    const account = await connectInstagram(tenant, "instagram")
    const tag = uniqueName("tag")
    const reply = await create(api, { name: `reply ${tag}` })
    const mention = await create(api, { name: `mention ${tag}`, trigger: "storyMention", inboxId: account.inboxId })
    const off = await create(api, { name: "off" })
    await api.patch(`/v1/ig-stories/${off.id}`, { isActive: false })

    const all = await api.get<{ data: IgStory[]; pageCount: number }>("/v1/ig-stories")
    expect(all.status).toBe(200)
    expect(all.json.pageCount).toBe(1)
    expect(all.json.data.map((item) => item.id).sort()).toEqual([reply.id, mention.id, off.id].sort())

    const ids = async (query: Record<string, string | number | boolean>) =>
      (await api.get<{ data: IgStory[] }>("/v1/ig-stories", { query })).json.data.map((item) => item.id).sort()
    expect(await ids({ trigger: "storyMention" })).toEqual([mention.id])
    expect(await ids({ trigger: "storyReply" })).toEqual([reply.id, off.id].sort())
    expect(await ids({ name: tag.toUpperCase() })).toEqual([reply.id, mention.id].sort())
    expect(await ids({ isActive: false })).toEqual([off.id])
    expect(await ids({ inboxId: account.inboxId })).toEqual([mention.id])

    const page = await api.get<{ data: IgStory[]; pageCount: number }>("/v1/ig-stories", { query: { perPage: 2 } })
    expect(page.json.pageCount).toBe(2)
    expect(page.json.data).toHaveLength(2)

    expectInvalid(await api.get("/v1/ig-stories", { query: { trigger: "storyLike" } }), "trigger")
  })
})

describe("igStories.delete / deleteMany", () => {
  it("delete answers 204, then 404; foreign ids are 404", async () => {
    const { api } = await tenantApi("p109-igs-delete")
    const created = await create(api)
    const response = await api.delete(`/v1/ig-stories/${created.id}`)
    expect(response.status).toBe(204)
    expect(response.text).toBe("")
    expectError(await api.get(`/v1/ig-stories/${created.id}`), 404, "notFound")
    expectError(await api.delete(`/v1/ig-stories/${created.id}`), 404, "notFound", NOT_FOUND)
    const { api: otherApi } = await tenantApi("p109-igs-delete-o")
    const foreign = await create(otherApi)
    expectError(await api.delete(`/v1/ig-stories/${foreign.id}`), 404, "notFound", NOT_FOUND)
    expect((await otherApi.get(`/v1/ig-stories/${foreign.id}`)).status).toBe(200)
  })

  it("deleteMany deletes own automations and ignores foreign and unknown ids (204)", async () => {
    const { api } = await tenantApi("p109-igs-bulk")
    const a = await create(api)
    const b = await create(api, { trigger: "storyMention" })
    const keep = await create(api)
    const { api: otherApi } = await tenantApi("p109-igs-bulk-o")
    const foreign = await create(otherApi)
    const response = await api.post("/v1/ig-stories/bulk-delete", { ids: [a.id, b.id, foreign.id, UNKNOWN_ID] })
    expect(response.status, response.text).toBe(204)
    const list = await api.get<{ data: IgStory[] }>("/v1/ig-stories")
    expect(list.json.data.map((item) => item.id)).toEqual([keep.id])
    expect((await otherApi.get(`/v1/ig-stories/${foreign.id}`)).status).toBe(200)
    expect((await api.post("/v1/ig-stories/bulk-delete", { ids: [] })).status).toBe(204)
    expectInvalid(await api.post("/v1/ig-stories/bulk-delete", { ids: [1.5] }), "ids.0")
  })
})

const STORY_FIELDS = "id,caption,media_type,media_url,thumbnail_url,timestamp,permalink"

describe("igStories.listStories", () => {
  it("variant instagram lists `me/stories`; a video story shows its media_url first", async () => {
    const { tenant, api } = await tenantApi("p109-igs-stories")
    const account = await connectInstagram(tenant, "instagram")
    const since = await fake.cursor()
    await fake.script({
      match: { host: "graph.instagram.com", method: "GET", pathRegex: graphPath("me/stories") },
      respond: {
        status: 200,
        json: {
          data: [
            {
              id: `${account.igId}9`,
              caption: "Story",
              media_type: "VIDEO",
              media_url: `https://scontent.cdninstagram.com/${account.igId}9.mp4`,
              thumbnail_url: `https://scontent.cdninstagram.com/${account.igId}9.jpg`,
              timestamp: "2026-01-05T08:00:00+0000",
              permalink: `https://www.instagram.com/stories/x/${account.igId}9/`,
            },
            { id: `${account.igId}8`, thumbnail_url: "https://scontent.cdninstagram.com/t.jpg", timestamp: "2026-01-05T07:00:00+0000" },
          ],
        },
      },
      times: 1,
    })
    const response = await api.get<Record<string, unknown>>("/v1/ig-stories/instagram-stories", {
      query: { variant: "instagram" },
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      stories: [
        {
          id: `${account.igId}9`,
          message: "Story",
          // Unlike igComments.listMedia, a video story's picture is the .mp4 URL.
          full_picture: `https://scontent.cdninstagram.com/${account.igId}9.mp4`,
          created_time: "2026-01-05T08:00:00+0000",
          permalink_url: `https://www.instagram.com/stories/x/${account.igId}9/`,
          accountId: account.igId,
          inboxId: account.inboxId,
        },
        {
          id: `${account.igId}8`,
          full_picture: "https://scontent.cdninstagram.com/t.jpg",
          created_time: "2026-01-05T07:00:00+0000",
          accountId: account.igId,
          inboxId: account.inboxId,
        },
      ],
      pages: [{ id: account.igId, name: account.name }],
    })
    const calls = await upstreamCalls({ host: "graph.instagram.com", since, path: /\/me\/stories$/ })
    expect(calls).toHaveLength(1)
    expect(calls[0]).toMatchObject({
      method: "GET",
      path: "/v23.0/me/stories",
      query: { fields: STORY_FIELDS, limit: "100" },
      headers: { authorization: `Bearer ig-token-${account.igId}` },
    })
  })

  it("variant instagramFacebook lists `<igId>/stories`; a failing account is left out; inboxId narrows", async () => {
    const { tenant, api } = await tenantApi("p109-igs-stories-fb")
    const ok = await connectInstagram(tenant, "facebook", "ok")
    const broken = await connectInstagram(tenant, "facebook", "broken")
    const since = await fake.cursor()
    await fake.script(
      {
        match: { host: "graph.facebook.com", method: "GET", pathRegex: graphPath(`${ok.igId}/stories`) },
        respond: { status: 200, json: { data: [{ id: "st-1", media_url: "https://scontent.cdninstagram.com/st-1.jpg", timestamp: "2026-01-06T00:00:00+0000" }] } },
      },
      {
        match: { host: "graph.facebook.com", method: "GET", pathRegex: graphPath(`${broken.igId}/stories`) },
        respond: { status: 400, json: { error: { message: "Invalid", type: "OAuthException", code: 190 } } },
      },
    )
    const response = await api.get<{ stories: unknown[]; pages: Array<{ id: string }> }>(
      "/v1/ig-stories/instagram-stories",
      { query: { variant: "instagramFacebook" } },
    )
    expect(response.status, response.text).toBe(200)
    expect(response.json.stories).toEqual([
      {
        id: "st-1",
        full_picture: "https://scontent.cdninstagram.com/st-1.jpg",
        created_time: "2026-01-06T00:00:00+0000",
        accountId: ok.igId,
        inboxId: ok.inboxId,
      },
    ])
    expect(response.json.pages.map((page) => page.id).sort()).toEqual([ok.igId, broken.igId].sort())
    const okCalls = await upstreamCalls({ host: "graph.facebook.com", since, path: new RegExp(`/${ok.igId}/stories$`) })
    expect(okCalls[0]).toMatchObject({
      path: `/v23.0/${ok.igId}/stories`,
      query: { fields: STORY_FIELDS, limit: "100" },
      headers: { authorization: `Bearer ig-token-${ok.igId}` },
    })

    const narrowed = await api.get("/v1/ig-stories/instagram-stories", {
      query: { variant: "instagramFacebook", inboxId: ok.inboxId },
    })
    expect(narrowed.json).toEqual({
      stories: response.json.stories,
      pages: [{ id: ok.igId, name: ok.name }],
    })
  })

  it("no account: empty lists; bad variant: 422", async () => {
    const { api } = await tenantApi("p109-igs-stories-none")
    const response = await api.get("/v1/ig-stories/instagram-stories", { query: { variant: "instagram" } })
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ stories: [], pages: [] })
    expectInvalid(await api.get("/v1/ig-stories/instagram-stories"), "variant")
    expectInvalid(await api.get("/v1/ig-stories/instagram-stories", { query: { variant: "tiktok" } }), "variant")
  })
})
