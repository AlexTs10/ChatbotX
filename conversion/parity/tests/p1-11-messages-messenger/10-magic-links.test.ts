// magicLinks.list / get / create / update / delete (scope `automation`).
// A magic link is a tracked short link `http://app/r/<workspaceId>/<name>`
// that redirects to its destination URL. The public resource never returns
// the destination: its `url` field is the shareable link (the destination is
// checked here through the redirect route `GET /r/<workspaceId>/<name>`).
// No mail is sent by these routes.

import { describe, expect, it } from "vitest"
import { HttpClient } from "../../src/http.ts"
import { Normalizer, unordered } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  type Api,
  expectError,
  expectInvalid,
  expectNotFound,
  ISO,
  keysOf,
  newTenant,
  SNOWFLAKE,
  UNKNOWN_ID,
} from "./helpers.ts"

type MagicLink = { id: string; name: string; url: string; createdAt: string; updatedAt: string }
type Page = { data: MagicLink[]; pageCount: number }

const KEYS = ["createdAt", "id", "name", "updatedAt", "url"]

const shareUrl = (workspaceId: string, name: string) => `http://app/r/${workspaceId}/${name}`

const create = async (api: Api, name: string, url: string): Promise<MagicLink> => {
  const response = await api.post<MagicLink>("/v1/magic-links", { name, url })
  expect(response.status, response.text).toBe(201)
  return response.json
}

/** Where the public redirect route sends a click. */
const redirectTarget = async (workspaceId: string, name: string, query = ""): Promise<{ status: number; location: string | null }> => {
  const response = await new HttpClient().get(`/r/${workspaceId}/${name}${query}`)
  return { status: response.status, location: response.headers.get("location") }
}

describe("magicLinks.create / get", () => {
  it("creates a link (201) whose `url` is the shareable link; the click redirects to the destination", async () => {
    const { tenant, api } = await newTenant("p111-ml-create")
    const n = new Normalizer()
    const name = `promo-${uniqueId()}`
    const destination = "https://shop.parity.test/landing?ref={{ref}}"
    const created = await create(api, name, destination)
    expect(keysOf(created)).toEqual(KEYS)
    expect(n.normalize(created)).toEqual({
      id: "<id:1>",
      name,
      url: shareUrl(tenant.workspaceId, name),
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })
    expect(created.id).toMatch(SNOWFLAKE)
    expect(created.createdAt).toMatch(ISO)

    const one = await api.get<MagicLink>(`/v1/magic-links/${created.id}`)
    expect(one.status, one.text).toBe(200)
    expect(one.json).toEqual(created)
    expect(one.text).not.toContain("shop.parity.test")

    expect(await redirectTarget(tenant.workspaceId, name, "?ref=abc")).toEqual({
      status: 302,
      location: "https://shop.parity.test/landing?ref=abc",
    })
  })

  it("names may hold slashes, dots, dashes and underscores", async () => {
    const { tenant, api } = await newTenant("p111-ml-path")
    const flat = `v1.2_x-${uniqueId()}`
    expect((await create(api, flat, "https://parity.test/flat")).url).toBe(shareUrl(tenant.workspaceId, flat))
    expect((await redirectTarget(tenant.workspaceId, flat)).location).toBe("https://parity.test/flat")
    // A name with `/` is accepted and gets a share URL, but the redirect route
    // has one `[name]` path segment, so that URL does not redirect.
    const nested = `campaign/${uniqueId()}/v1`
    const created = await create(api, nested, "https://parity.test/a")
    expect(created.url).toBe(shareUrl(tenant.workspaceId, nested))
    expect((await redirectTarget(tenant.workspaceId, nested)).status).toBe(404)
  })

  it("validates name and url (422) and refuses a taken name (422 validation)", async () => {
    const { api } = await newTenant("p111-ml-invalid")
    const name = `dup-${uniqueId()}`
    await create(api, name, "https://parity.test/1")
    const taken = await api.post("/v1/magic-links", { name, url: "https://parity.test/2" })
    expectError(taken, 422, "validation", { message: "Name is already taken" })
    expectInvalid(await api.post("/v1/magic-links", { name: "has space", url: "https://parity.test" }), "name")
    expectInvalid(await api.post("/v1/magic-links", { name: "/leading", url: "https://parity.test" }), "name")
    expectInvalid(await api.post("/v1/magic-links", { name: "x".repeat(129), url: "https://parity.test" }), "name")
    expectInvalid(await api.post("/v1/magic-links", { name: "ok", url: "not a url" }), "url")
    expectInvalid(await api.post("/v1/magic-links", { name: "ok", url: `https://parity.test/${"a".repeat(1000)}` }), "url")
    expectInvalid(await api.post("/v1/magic-links", { url: "https://parity.test" }), "name")
    expectInvalid(await api.post("/v1/magic-links", { name: "ok" }), "url")
  })

  it("the same name is free in another workspace", async () => {
    const { api } = await newTenant("p111-ml-other-a")
    const { tenant: otherTenant, api: otherApi } = await newTenant("p111-ml-other-b")
    const name = `shared-${uniqueId()}`
    await create(api, name, "https://parity.test/a")
    const other = await create(otherApi, name, "https://parity.test/b")
    expect(other.url).toBe(shareUrl(otherTenant.workspaceId, name))
    expect((await redirectTarget(otherTenant.workspaceId, name)).location).toBe("https://parity.test/b")
  })

  it("get: unknown or foreign id -> 404; non-numeric id -> 422", async () => {
    const { api } = await newTenant("p111-ml-get-404")
    const created = await create(api, `g-${uniqueId()}`, "https://parity.test")
    const { api: otherApi } = await newTenant("p111-ml-get-404-o")
    expectNotFound(await otherApi.get(`/v1/magic-links/${created.id}`), "Magic link not found")
    expectNotFound(await api.get(`/v1/magic-links/${UNKNOWN_ID}`), "Magic link not found")
    expectInvalid(await api.get("/v1/magic-links/abc"), "id")
  })
})

describe("magicLinks.list", () => {
  it("lists the workspace's links with paging and keyword search over name and destination", async () => {
    const { tenant, api } = await newTenant("p111-ml-list")
    expect((await api.get<Page>("/v1/magic-links")).json).toEqual({ data: [], pageCount: 0 })
    const marker = uniqueId()
    const a = await create(api, `alpha-${marker}`, "https://parity.test/one")
    const b = await create(api, `beta-${marker}`, "https://dest-findme.parity.test/two")
    const c = await create(api, `gamma-${marker}`, "https://parity.test/three")

    const all = await api.get<Page>("/v1/magic-links")
    expect(all.status, all.text).toBe(200)
    expect(keysOf(all.json)).toEqual(["data", "pageCount"])
    expect(all.json.pageCount).toBe(1)
    expect(unordered(all.json.data, (item) => item.id)).toEqual(unordered([a, b, c], (item) => item.id))
    for (const item of all.json.data) {
      expect(item.url).toBe(shareUrl(tenant.workspaceId, item.name))
    }

    const paged = await api.get<Page>("/v1/magic-links", { query: { perPage: 2, page: 1 } })
    expect(paged.json.pageCount).toBe(2)
    expect(paged.json.data).toHaveLength(2)
    const second = await api.get<Page>("/v1/magic-links", { query: { perPage: 2, page: 2 } })
    expect(second.json.data).toHaveLength(1)
    expect(
      new Set([...paged.json.data, ...second.json.data].map((item) => item.id)),
    ).toEqual(new Set([a.id, b.id, c.id]))

    // `keyword` matches the name or the (hidden) destination, case-insensitively.
    const byName = await api.get<Page>("/v1/magic-links", { query: { keyword: `ALPHA-${marker}` } })
    expect(byName.json.data.map((item) => item.id)).toEqual([a.id])
    const byDestination = await api.get<Page>("/v1/magic-links", { query: { keyword: "DEST-FINDME" } })
    expect(byDestination.json.data.map((item) => item.id)).toEqual([b.id])
    expect((await api.get<Page>("/v1/magic-links", { query: { keyword: "no-such-link" } })).json).toEqual({
      data: [],
      pageCount: 0,
    })

    // Another workspace sees none of them.
    const { api: otherApi } = await newTenant("p111-ml-list-o")
    expect((await otherApi.get<Page>("/v1/magic-links")).json.data).toEqual([])
  })

  it("sort by name ascending and descending", async () => {
    const { api } = await newTenant("p111-ml-sort")
    const marker = uniqueId()
    await create(api, `b-${marker}`, "https://parity.test/b")
    await create(api, `a-${marker}`, "https://parity.test/a")
    await create(api, `c-${marker}`, "https://parity.test/c")
    const asc = await api.get<Page>("/v1/magic-links?sort[0][id]=name&sort[0][desc]=false")
    expect(asc.status, asc.text).toBe(200)
    expect(asc.json.data.map((item) => item.name)).toEqual([`a-${marker}`, `b-${marker}`, `c-${marker}`])
    const desc = await api.get<Page>("/v1/magic-links?sort[0][id]=name&sort[0][desc]=true")
    expect(desc.json.data.map((item) => item.name)).toEqual([`c-${marker}`, `b-${marker}`, `a-${marker}`])
  })

  it("perPage above 50 or below 1 and page 0 are 422", async () => {
    const { api } = await newTenant("p111-ml-list-invalid")
    expectInvalid(await api.get("/v1/magic-links", { query: { perPage: 51 } }), "perPage")
    expectInvalid(await api.get("/v1/magic-links", { query: { perPage: 0 } }), "perPage")
    expectInvalid(await api.get("/v1/magic-links", { query: { page: 0 } }), "page")
  })
})

describe("magicLinks.update", () => {
  it("renames and repoints a link; the old name stops redirecting", async () => {
    const { tenant, api } = await newTenant("p111-ml-update")
    const oldName = `old-${uniqueId()}`
    const created = await create(api, oldName, "https://parity.test/old")
    const newName = `new-${uniqueId()}`
    const renamed = await api.patch<MagicLink>(`/v1/magic-links/${created.id}`, { name: newName })
    expect(renamed.status, renamed.text).toBe(200)
    expect(renamed.json).toMatchObject({ id: created.id, name: newName, url: shareUrl(tenant.workspaceId, newName) })
    expect(keysOf(renamed.json)).toEqual(KEYS)
    expect((await redirectTarget(tenant.workspaceId, newName)).location).toBe("https://parity.test/old")
    expect((await redirectTarget(tenant.workspaceId, oldName)).status).toBe(404)

    const repointed = await api.patch<MagicLink>(`/v1/magic-links/${created.id}`, { url: "https://parity.test/new" })
    expect(repointed.status, repointed.text).toBe(200)
    expect(repointed.json.name).toBe(newName)
    expect((await redirectTarget(tenant.workspaceId, newName)).location).toBe("https://parity.test/new")

    // An empty patch fails with a 500 (the update has no column to set) and changes nothing.
    const empty = await api.patch<MagicLink>(`/v1/magic-links/${created.id}`, {})
    expectError(empty, 500, "INTERNAL_SERVER_ERROR", { defined: false })
    expect((await api.get<MagicLink>(`/v1/magic-links/${created.id}`)).json.name).toBe(newName)
  })

  it("a taken name is 422 validation; unknown id 404; bad input 422", async () => {
    const { api } = await newTenant("p111-ml-update-err")
    const a = await create(api, `a-${uniqueId()}`, "https://parity.test/a")
    const b = await create(api, `b-${uniqueId()}`, "https://parity.test/b")
    expectError(await api.patch(`/v1/magic-links/${b.id}`, { name: a.name }), 422, "validation", {
      message: "Name is already taken",
    })
    expectNotFound(await api.patch(`/v1/magic-links/${UNKNOWN_ID}`, { name: "x" }), "Magic link not found")
    expectInvalid(await api.patch(`/v1/magic-links/${b.id}`, { url: "nope" }), "url")
    expectInvalid(await api.patch(`/v1/magic-links/${b.id}`, { name: "bad name" }), "name")
    const { api: otherApi } = await newTenant("p111-ml-update-err-o")
    expectNotFound(await otherApi.patch(`/v1/magic-links/${a.id}`, { name: "x" }), "Magic link not found")
  })
})

describe("magicLinks.delete", () => {
  it("deletes (204); the link stops redirecting; a second delete is 404", async () => {
    const { tenant, api } = await newTenant("p111-ml-delete")
    const name = `del-${uniqueId()}`
    const created = await create(api, name, "https://parity.test/gone")
    const response = await api.delete(`/v1/magic-links/${created.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectNotFound(await api.get(`/v1/magic-links/${created.id}`), "Magic link not found")
    expect((await redirectTarget(tenant.workspaceId, name)).status).toBe(404)
    expectNotFound(await api.delete(`/v1/magic-links/${created.id}`), "Magic link not found")
    expectInvalid(await api.delete("/v1/magic-links/abc"), "id")
  })

  it("another workspace cannot delete the link", async () => {
    const { tenant, api } = await newTenant("p111-ml-delete-o")
    const name = `keep-${uniqueId()}`
    const created = await create(api, name, "https://parity.test/kept")
    const { api: otherApi } = await newTenant("p111-ml-delete-o2")
    expectNotFound(await otherApi.delete(`/v1/magic-links/${created.id}`), "Magic link not found")
    expect((await redirectTarget(tenant.workspaceId, name)).location).toBe("https://parity.test/kept")
  })
})
