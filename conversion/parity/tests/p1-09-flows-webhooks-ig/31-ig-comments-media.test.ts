// igComments.listMedia (Graph API media of the connected Instagram accounts)
// and the Live-comments subscription that igComments.create makes for an
// Instagram Login Live automation. Every Graph call is scripted on the fake
// upstream: graph.instagram.com (Instagram Login, `me/media`) and
// graph.facebook.com (via Facebook Login, `<igId>/media`).

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import {
  connectInstagram,
  DEFAULT_HIDE,
  DEFAULT_OPTIONS,
  expectInvalid,
  graphPath,
  igCommentBody,
  tenantApi,
  upstreamCalls,
} from "./helpers-ig.ts"

const MEDIA_FIELDS = "id,caption,media_product_type,media_type,media_url,thumbnail_url,timestamp,permalink"

type MediaList = {
  posts: Array<Record<string, unknown>>
  pages: Array<{ id: string; name: string }>
}

const mediaItems = (prefix: string) => [
  {
    id: `${prefix}1`,
    caption: "Image post",
    media_type: "IMAGE",
    media_product_type: "FEED",
    media_url: `https://scontent.cdninstagram.com/${prefix}1.jpg`,
    timestamp: "2026-01-02T10:00:00+0000",
    permalink: `https://www.instagram.com/p/${prefix}1/`,
  },
  {
    id: `${prefix}2`,
    caption: "Reel",
    media_type: "VIDEO",
    media_product_type: "REELS",
    media_url: `https://scontent.cdninstagram.com/${prefix}2.mp4`,
    thumbnail_url: `https://scontent.cdninstagram.com/${prefix}2-thumb.jpg`,
    timestamp: "2026-01-03T10:00:00+0000",
    permalink: `https://www.instagram.com/reel/${prefix}2/`,
  },
  {
    // A video without a thumbnail shows no image; a bare item keeps only its id and time.
    id: `${prefix}3`,
    media_type: "VIDEO",
    media_url: `https://scontent.cdninstagram.com/${prefix}3.mp4`,
    timestamp: "2026-01-04T10:00:00+0000",
  },
]

describe("igComments.listMedia", () => {
  it("variant instagram: lists `me/media` of the Instagram Login account", async () => {
    const { tenant, api } = await tenantApi("p109-igm-login")
    const account = await connectInstagram(tenant, "instagram")
    const since = await fake.cursor()
    await fake.script({
      match: { host: "graph.instagram.com", method: "GET", pathRegex: graphPath("me/media") },
      respond: { status: 200, json: { data: mediaItems(account.igId) } },
      times: 1,
    })
    const response = await api.get<MediaList>("/v1/ig-comments/instagram-media", { query: { variant: "instagram" } })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      posts: [
        {
          id: `${account.igId}1`,
          message: "Image post",
          full_picture: `https://scontent.cdninstagram.com/${account.igId}1.jpg`,
          created_time: "2026-01-02T10:00:00+0000",
          permalink_url: `https://www.instagram.com/p/${account.igId}1/`,
          media_product_type: "FEED",
          accountId: account.igId,
          inboxId: account.inboxId,
        },
        {
          id: `${account.igId}2`,
          message: "Reel",
          // A video shows its thumbnail, never the .mp4.
          full_picture: `https://scontent.cdninstagram.com/${account.igId}2-thumb.jpg`,
          created_time: "2026-01-03T10:00:00+0000",
          permalink_url: `https://www.instagram.com/reel/${account.igId}2/`,
          media_product_type: "REELS",
          accountId: account.igId,
          inboxId: account.inboxId,
        },
        {
          id: `${account.igId}3`,
          created_time: "2026-01-04T10:00:00+0000",
          accountId: account.igId,
          inboxId: account.inboxId,
        },
      ],
      pages: [{ id: account.igId, name: account.name }],
    })
    const [call, ...rest] = await upstreamCalls({ host: "graph.instagram.com", since, path: /\/me\/media$/ })
    expect(rest).toEqual([])
    expect(call).toMatchObject({
      method: "GET",
      path: "/v23.0/me/media",
      query: { fields: MEDIA_FIELDS, limit: "100" },
      headers: { authorization: `Bearer ig-token-${account.igId}` },
    })
  })

  it("variant instagramFacebook: one call per account; a failing account keeps its page but no posts", async () => {
    const { tenant, api } = await tenantApi("p109-igm-fb")
    const ok = await connectInstagram(tenant, "facebook", "ok")
    const broken = await connectInstagram(tenant, "facebook", "broken")
    // An Instagram Login account is not part of the instagramFacebook variant.
    await connectInstagram(tenant, "instagram", "login")
    const since = await fake.cursor()
    await fake.script(
      {
        match: { host: "graph.facebook.com", method: "GET", pathRegex: graphPath(`${ok.igId}/media`) },
        respond: { status: 200, json: { data: mediaItems(ok.igId).slice(0, 1) } },
      },
      {
        match: { host: "graph.facebook.com", method: "GET", pathRegex: graphPath(`${broken.igId}/media`) },
        respond: {
          status: 400,
          json: { error: { message: "Unsupported get request", type: "GraphMethodException", code: 100 } },
        },
      },
    )
    const response = await api.get<MediaList>("/v1/ig-comments/instagram-media", {
      query: { variant: "instagramFacebook" },
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json.posts).toEqual([
      {
        id: `${ok.igId}1`,
        message: "Image post",
        full_picture: `https://scontent.cdninstagram.com/${ok.igId}1.jpg`,
        created_time: "2026-01-02T10:00:00+0000",
        permalink_url: `https://www.instagram.com/p/${ok.igId}1/`,
        media_product_type: "FEED",
        accountId: ok.igId,
        inboxId: ok.inboxId,
      },
    ])
    expect([...response.json.pages].sort((a, b) => a.id.localeCompare(b.id))).toEqual(
      [
        { id: ok.igId, name: ok.name },
        { id: broken.igId, name: broken.name },
      ].sort((a, b) => a.id.localeCompare(b.id)),
    )
    const okCall = await upstreamCalls({ host: "graph.facebook.com", since, path: new RegExp(`/${ok.igId}/media$`) })
    expect(okCall).toHaveLength(1)
    expect(okCall[0]).toMatchObject({
      path: `/v23.0/${ok.igId}/media`,
      query: { fields: MEDIA_FIELDS, limit: "100" },
      headers: { authorization: `Bearer ig-token-${ok.igId}` },
    })
    // A 400 is not retried.
    expect(
      await upstreamCalls({ host: "graph.facebook.com", since, path: new RegExp(`/${broken.igId}/media$`) }),
    ).toHaveLength(1)
    expect(await upstreamCalls({ host: "graph.instagram.com", since })).toEqual([])

    // `inboxId` narrows to one account.
    const narrowed = await api.get<MediaList>("/v1/ig-comments/instagram-media", {
      query: { variant: "instagramFacebook", inboxId: broken.inboxId },
    })
    expect(narrowed.status).toBe(200)
    expect(narrowed.json).toEqual({ posts: [], pages: [{ id: broken.igId, name: broken.name }] })
  })

  it("no connected account: empty lists and no Graph call", async () => {
    const { api } = await tenantApi("p109-igm-none")
    const since = await fake.cursor()
    for (const variant of ["instagram", "instagramFacebook"]) {
      const response = await api.get<MediaList>("/v1/ig-comments/instagram-media", { query: { variant } })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({ posts: [], pages: [] })
    }
    expect(await upstreamCalls({ host: "graph.instagram.com", since })).toEqual([])
    expect(await upstreamCalls({ host: "graph.facebook.com", since })).toEqual([])
  })

  it("validates the query (422 invalidRequestData)", async () => {
    const { api } = await tenantApi("p109-igm-invalid")
    expectInvalid(await api.get("/v1/ig-comments/instagram-media"), "variant")
    expectInvalid(await api.get("/v1/ig-comments/instagram-media", { query: { variant: "messenger" } }), "variant")
    expectInvalid(
      await api.get("/v1/ig-comments/instagram-media", { query: { variant: "instagram", inboxId: "abc" } }),
      "inboxId",
    )
  })
})

describe("igComments.create: Instagram Live subscription", () => {
  it("re-subscribes every Instagram Login account with `live_comments` and pins Live-only settings", async () => {
    const { tenant, api } = await tenantApi("p109-igm-live")
    const first = await connectInstagram(tenant, "instagram", "live-a")
    const second = await connectInstagram(tenant, "instagram", "live-b")
    const viaFacebook = await connectInstagram(tenant, "facebook", "live-fb")
    const since = await fake.cursor()
    await fake.script(
      ...[first, second].map((account) => ({
        match: {
          host: "graph.instagram.com",
          method: "POST",
          pathRegex: graphPath(`${account.igId}/subscribed_apps`),
        },
        respond: { status: 200, json: { success: true } },
      })),
    )
    const response = await api.post<Record<string, unknown>>(
      "/v1/ig-comments",
      igCommentBody({
        post: { type: "live", value: [] },
        publicReply: { type: "text", value: "Public" },
        options: { ...DEFAULT_OPTIONS, likeUserComment: true, ignoreCommentReplies: true },
        hideComments: { ...DEFAULT_HIDE, hasPhoneNumber: true, hasEmoji: true, showCommentsAfter: "2d" },
        replyAfter: { type: "randomWithin3Minutes", value: 0 },
      }),
    )
    expect(response.status, response.text).toBe(201)
    expect(response.json).toMatchObject({
      post: { type: "live", value: [] },
      privateReply: { type: "text", value: "Thanks for your comment" },
      publicReply: { type: "none", value: null },
      options: { ...DEFAULT_OPTIONS, likeUserComment: false, ignoreCommentReplies: false },
      hideComments: {
        ...DEFAULT_HIDE,
        hasPhoneNumber: false,
        hasGif: false,
        hasEmoji: false,
        showCommentsAfter: "none",
      },
      replyAfter: { type: "immediately", value: 0 },
    })

    const calls = await upstreamCalls({ host: "graph.instagram.com", since, path: /\/subscribed_apps$/ })
    expect(calls.map((call) => call.path).sort()).toEqual(
      [`/v23.0/${first.igId}/subscribed_apps`, `/v23.0/${second.igId}/subscribed_apps`].sort(),
    )
    for (const call of calls) {
      const account = call.path.includes(first.igId) ? first : second
      expect(call.method).toBe("POST")
      expect(call.headers.authorization).toBe(`Bearer ig-token-${account.igId}`)
      expect(JSON.parse(call.bodyText)).toEqual({
        subscribed_fields:
          "messages,messaging_postbacks,messaging_optins,messaging_seen,messaging_referral,comments,live_comments",
      })
    }
    // The Facebook Login account is never re-subscribed.
    expect(await upstreamCalls({ host: "graph.facebook.com", since })).toEqual([])
    expect(calls.some((call) => call.path.includes(viaFacebook.igId))).toBe(false)
  })

  it("a failing subscription still saves the automation (best-effort)", async () => {
    const { tenant, api } = await tenantApi("p109-igm-live-fail")
    const account = await connectInstagram(tenant, "instagram")
    const since = await fake.cursor()
    await fake.script({
      match: { host: "graph.instagram.com", method: "POST", pathRegex: graphPath(`${account.igId}/subscribed_apps`) },
      respond: { status: 200, json: { success: false } },
    })
    const response = await api.post<{ id: string }>("/v1/ig-comments", igCommentBody({ post: { type: "live", value: [] } }))
    expect(response.status, response.text).toBe(201)
    expect((await api.get(`/v1/ig-comments/${response.json.id}`)).status).toBe(200)
    expect(await upstreamCalls({ host: "graph.instagram.com", since, path: /\/subscribed_apps$/ })).toHaveLength(1)
  })

  it("an instagramFacebook Live automation makes no subscription call", async () => {
    const { tenant, api } = await tenantApi("p109-igm-live-fb")
    await connectInstagram(tenant, "facebook")
    await connectInstagram(tenant, "instagram")
    const since = await fake.cursor()
    const response = await api.post(
      "/v1/ig-comments",
      igCommentBody({ type: "instagramFacebook", post: { type: "live", value: [] } }),
    )
    expect(response.status, response.text).toBe(201)
    expect(await upstreamCalls({ host: "graph.instagram.com", since })).toEqual([])
    expect(await upstreamCalls({ host: "graph.facebook.com", since })).toEqual([])
  })
})
