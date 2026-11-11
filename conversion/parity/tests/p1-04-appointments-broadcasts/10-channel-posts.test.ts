// channelPosts.list, channelPosts.listSelected

import { describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import { bracketArray, expectInvalid, ext, newTenant } from "./helpers.ts"

type Post = {
  id: string
  caption: string | null
  channel: string
  externalPostId: string
  inboxId: string
  inboxName: string
  permalink: string | null
  publishedAt: string | null
  thumbnailUrl: string | null
}
type Page = { items: Post[]; nextCursor?: { id: string; sortAt: string } }

/** PostgreSQL `timestamptz` text or ISO 8601 (the cursor accepts both). */
const SORT_AT = /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2}):(\d{2})(?:\.\d{1,6})?(?:Z|[+-]\d{2}(?::?\d{2})?)$/
const instantOf = (sortAt: string): number => {
  const iso = sortAt.replace(" ", "T").replace(/([+-]\d{2})$/, "$1:00")
  return Date.parse(iso)
}

const seedPosts = async (tenant: Tenant) => {
  const fixture = await ext()
  const { inboxId } = await fixture.createInbox({
    workspaceId: tenant.workspaceId,
    channel: "instagram",
    name: "Parity IG",
    sourceId: `ig-${uniqueId()}`,
  })
  const post = (externalPostId: string, caption: string | null, publishedAt: Date | null, thumbnail: string | null) =>
    fixture.createChannelPost({
      workspaceId: tenant.workspaceId,
      inboxId,
      channel: "instagram",
      integrationId: "1",
      sourceAccountId: "parity-account",
      externalPostId,
      caption,
      permalink: `https://www.instagram.com/p/${externalPostId}/`,
      thumbnail,
      publishedAt,
    })
  const jan1 = await post("post-1", "Summer sale", new Date("2026-01-01T10:00:00.000Z"), null)
  const jan2 = await post("post-2", "Winter SALE", new Date("2026-01-02T10:00:00.000Z"), "media/thumb-2.jpg")
  const jan3 = await post("post-3", null, new Date("2026-01-03T10:00:00.000Z"), null)
  return { inboxId, ids: [jan1.channelPostId, jan2.channelPostId, jan3.channelPostId] }
}

describe("channelPosts.list", () => {
  it("is empty for a new workspace", async () => {
    const { api } = await newTenant("posts-empty")
    const response = await api.get("/v1/channel-posts")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ items: [] })
  })

  it("lists posts newest first with inbox and public thumbnail URL; search by caption", async () => {
    const normalizer = new Normalizer()
    const { tenant, api } = await newTenant("posts-list")
    const { inboxId, ids } = await seedPosts(tenant)
    const response = await api.get<Page>("/v1/channel-posts")
    expect(response.status, response.text).toBe(200)
    expect(response.json.nextCursor).toBeUndefined()
    expect(response.json.items.map((item) => item.id)).toEqual([ids[2], ids[1], ids[0]])
    expect(response.json.items[1]).toEqual({
      id: ids[1],
      caption: "Winter SALE",
      channel: "instagram",
      externalPostId: "post-2",
      inboxId,
      inboxName: "Parity IG",
      permalink: "https://www.instagram.com/p/post-2/",
      publishedAt: "2026-01-02T10:00:00.000Z",
      thumbnailUrl: "http://app/storage/media/thumb-2.jpg",
    })
    expect(normalizer.normalize(response.json, { keep: ["publishedAt"] })).toMatchSnapshot("list")

    const search = await api.get<Page>("/v1/channel-posts", { query: { search: "sale" } })
    expect(search.json.items.map((item) => item.id)).toEqual([ids[1], ids[0]])
  })

  it("pages with the keyset cursor", async () => {
    const { tenant, api } = await newTenant("posts-cursor")
    const { ids } = await seedPosts(tenant)
    const first = await api.get<Page>("/v1/channel-posts", { query: { limit: 2 } })
    expect(first.status, first.text).toBe(200)
    expect(first.json.items.map((item) => item.id)).toEqual([ids[2], ids[1]])
    const cursor = first.json.nextCursor
    expect(cursor?.id).toBe(ids[1])
    expect(cursor?.sortAt).toMatch(SORT_AT)
    expect(instantOf(cursor?.sortAt ?? "")).toBe(Date.parse("2026-01-02T10:00:00.000Z"))
    expect(Object.keys(cursor ?? {}).sort()).toEqual(["id", "sortAt"])

    const query = `cursor%5Bid%5D=${cursor?.id}&cursor%5BsortAt%5D=${encodeURIComponent(cursor?.sortAt ?? "")}&limit=2`
    const second = await api.get<Page>(`/v1/channel-posts?${query}`)
    expect(second.status, second.text).toBe(200)
    expect(second.json.items.map((item) => item.id)).toEqual([ids[0]])
    expect(second.json.nextCursor).toBeUndefined()
  })

  it("validates limit, search and cursor (422)", async () => {
    const { api } = await newTenant("posts-invalid")
    expectInvalid(await api.get("/v1/channel-posts", { query: { limit: 0 } }), "limit")
    expectInvalid(await api.get("/v1/channel-posts", { query: { limit: 101 } }), "limit")
    expectInvalid(await api.get("/v1/channel-posts", { query: { search: "x".repeat(201) } }), "search")
    expectInvalid(await api.get("/v1/channel-posts", { query: { search: "   " } }), "search")
    expectInvalid(await api.get("/v1/channel-posts?cursor%5Bid%5D=0&cursor%5BsortAt%5D=2026-01-01T00%3A00%3A00Z"), "cursor.id")
    expectInvalid(await api.get("/v1/channel-posts?cursor%5Bid%5D=5&cursor%5BsortAt%5D=yesterday"), "cursor.sortAt")
  })
})

describe("channelPosts.listSelected", () => {
  it("resolves the workspace's posts by id and drops unknown and foreign ids", async () => {
    const normalizer = new Normalizer()
    const { tenant, api } = await newTenant("posts-selected")
    const { ids } = await seedPosts(tenant)
    const { tenant: otherTenant } = await newTenant("posts-selected-other")
    const { ids: foreignIds } = await seedPosts(otherTenant)

    const response = await api.get<{ data: Post[] }>(
      `/v1/channel-posts/options/by-ids?${bracketArray("ids", [ids[0] as string, ids[2] as string, foreignIds[0] as string, "999"])}`,
    )
    expect(response.status, response.text).toBe(200)
    const found = response.json.data.map((item) => item.id).sort()
    expect(found).toEqual([ids[0], ids[2]].sort())
    const sorted = [...response.json.data].sort((a, b) => a.externalPostId.localeCompare(b.externalPostId))
    expect(normalizer.normalize({ data: sorted }, { keep: ["publishedAt"] })).toMatchSnapshot("selected")
  })

  it("rejects a missing, duplicate, oversized or non-id list (422)", async () => {
    const { api } = await newTenant("posts-selected-invalid")
    expectInvalid(await api.get("/v1/channel-posts/options/by-ids"), "ids")
    expectInvalid(await api.get(`/v1/channel-posts/options/by-ids?${bracketArray("ids", ["5", "5"])}`), "ids")
    expectInvalid(
      await api.get(
        `/v1/channel-posts/options/by-ids?${bracketArray("ids", Array.from({ length: 101 }, (_, index) => String(index + 1)))}`,
      ),
      "ids",
    )
    expectInvalid(await api.get(`/v1/channel-posts/options/by-ids?${bracketArray("ids", ["0"])}`), "ids.0")
    expectInvalid(
      await api.get(`/v1/channel-posts/options/by-ids?${bracketArray("ids", ["99999999999999999999"])}`),
      "ids.0",
    )
  })
})
