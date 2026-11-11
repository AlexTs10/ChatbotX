// inboxes.list (GET /v1/inboxes), inboxes.listChannels (deprecated GET
// /v1/channels) and inboxes.update (PATCH /v1/inboxes/{id}). Inbox rows come
// from the P1.10 fixture extension (no integration behind them).

import { describe, expect, it } from "vitest"
import { unordered } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import { type Api, expectInvalid, expectNotFound, ext, newTenant, SNOWFLAKE, UNKNOWN_ID, uniqueName } from "./helpers.ts"

type Inbox = {
  id: string
  name: string
  channel: string
  status: string
  sourceId: string
  markReadOnOutbound: boolean
}
type Listed = { data: Inbox[]; pageCount: number }

const INBOX_KEYS = ["channel", "id", "markReadOnOutbound", "name", "sourceId", "status"]

const addInboxes = async (workspaceId: string, count: number, channel = "telegram") => {
  const fx = await ext()
  const created: Array<{ id: string; name: string; sourceId: string }> = []
  for (let index = 0; index < count; index++) {
    const name = uniqueName(`Inbox ${index}`)
    const sourceId = `src-${uniqueId()}`
    const { inboxId } = await fx.createInbox({ workspaceId, channel, name, sourceId })
    created.push({ id: inboxId, name, sourceId })
  }
  return created
}

const list = async (api: Api, query: Record<string, string | number> = {}, path = "/v1/inboxes") => {
  const response = await api.get<Listed>(path, { query })
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("inboxes.list", () => {
  it("is empty for a new workspace", async () => {
    const { api } = await newTenant("p110-inbox-empty")
    expect(await list(api)).toEqual({ data: [], pageCount: 0 })
  })

  it("lists connected inboxes only, with the public fields", async () => {
    const { tenant, api } = await newTenant("p110-inbox-list")
    const fx = await ext()
    const [telegram] = await addInboxes(tenant.workspaceId, 1, "telegram")
    const [zalo] = await addInboxes(tenant.workspaceId, 1, "zalo")
    await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "webchat", name: "Gone", status: "disconnected" })
    const result = await list(api)
    expect(result.pageCount).toBe(1)
    for (const inbox of result.data) {
      expect(Object.keys(inbox).sort()).toEqual(INBOX_KEYS)
    }
    expect(unordered(result.data, (inbox) => inbox.id)).toEqual(
      unordered(
        [
          { id: telegram?.id, name: telegram?.name, channel: "telegram", status: "connected", sourceId: telegram?.sourceId, markReadOnOutbound: false },
          { id: zalo?.id, name: zalo?.name, channel: "zalo", status: "connected", sourceId: zalo?.sourceId, markReadOnOutbound: false },
        ] as Inbox[],
        (inbox) => inbox.id,
      ),
    )
    expect(result.data.every((inbox) => SNOWFLAKE.test(inbox.id))).toBe(true)
  })

  it("does not list another workspace's inboxes", async () => {
    const { api } = await newTenant("p110-inbox-iso")
    const { tenant: other } = await newTenant("p110-inbox-iso-o")
    await addInboxes(other.workspaceId, 1)
    expect((await list(api)).data).toEqual([])
  })

  it("pages: no perPage returns up to 20 rows but counts pages of 10; perPage caps at 50", async () => {
    const { tenant, api } = await newTenant("p110-inbox-page")
    const created = await addInboxes(tenant.workspaceId, 11)
    const ids = created.map((inbox) => inbox.id).sort()

    // Default: limit 20 (all 11 rows), pageCount computed with 10 per page.
    const all = await list(api)
    expect(all.data.map((inbox) => inbox.id).sort()).toEqual(ids)
    expect(all.pageCount).toBe(2)

    const pages: string[] = []
    for (const page of [1, 2, 3]) {
      const result = await list(api, { page, perPage: 5 })
      expect(result.pageCount).toBe(3)
      expect(result.data).toHaveLength(page === 3 ? 1 : 5)
      pages.push(...result.data.map((inbox) => inbox.id))
    }
    expect(pages.sort()).toEqual(ids)

    // perPage above 50 is clamped to 50 rows; pageCount uses the asked size.
    const wide = await list(api, { perPage: 60 })
    expect(wide.data).toHaveLength(11)
    expect(wide.pageCount).toBe(1)

    // `page` alone pages by 20 rows (page 2 is empty here) but still counts pages of 10.
    const second = await list(api, { page: 2 })
    expect(second).toEqual({ data: [], pageCount: 2 })
  })

  it("accepts includes[]=integration without changing the public shape", async () => {
    const { tenant, api } = await newTenant("p110-inbox-incl")
    await addInboxes(tenant.workspaceId, 1)
    const response = await api.get<Listed>("/v1/inboxes?includes[]=integration")
    expect(response.status, response.text).toBe(200)
    expect(response.json.data).toHaveLength(1)
    expect(Object.keys(response.json.data[0] as Inbox).sort()).toEqual(INBOX_KEYS)
  })

  it("validates the query (422 invalidRequestData)", async () => {
    const { api } = await newTenant("p110-inbox-invalid")
    expectInvalid(await api.get("/v1/inboxes", { query: { perPage: 0 } }), "perPage")
    expectInvalid(await api.get("/v1/inboxes", { query: { page: 0 } }), "page")
    expectInvalid(await api.get("/v1/inboxes", { query: { page: "x" } }), "page")
    expectInvalid(await api.get("/v1/inboxes?includes[]=contacts"), "includes.0")
  })
})

describe("inboxes.listChannels (deprecated)", () => {
  it("returns sourceId as id and drops sourceId / markReadOnOutbound", async () => {
    const { tenant, api } = await newTenant("p110-channels")
    const [inbox] = await addInboxes(tenant.workspaceId, 1, "zalo")
    const result = await list(api, {}, "/v1/channels")
    expect(result).toEqual({
      data: [{ id: inbox?.sourceId, name: inbox?.name, channel: "zalo", status: "connected" }],
      pageCount: 1,
    })
  })

  it("pages like inboxes.list", async () => {
    const { tenant, api } = await newTenant("p110-channels-page")
    await addInboxes(tenant.workspaceId, 3)
    const result = await list(api, { page: 2, perPage: 2 }, "/v1/channels")
    expect(result.pageCount).toBe(2)
    expect(result.data).toHaveLength(1)
    expectInvalid(await api.get("/v1/channels", { query: { perPage: -1 } }), "perPage")
  })
})

describe("inboxes.update", () => {
  it("sets markReadOnOutbound and returns the inbox; list shows the new value", async () => {
    const { tenant, api } = await newTenant("p110-inbox-update")
    const [inbox] = await addInboxes(tenant.workspaceId, 1)
    const id = inbox?.id as string
    const on = await api.patch<Inbox>(`/v1/inboxes/${id}`, { markReadOnOutbound: true })
    expect(on.status, on.text).toBe(200)
    expect(on.json).toEqual({
      id,
      name: inbox?.name,
      channel: "telegram",
      status: "connected",
      sourceId: inbox?.sourceId,
      markReadOnOutbound: true,
    })
    expect((await list(api)).data[0]?.markReadOnOutbound).toBe(true)

    const off = await api.patch<Inbox>(`/v1/inboxes/${id}`, { markReadOnOutbound: false })
    expect(off.json.markReadOnOutbound).toBe(false)
  })

  it("an empty body returns the inbox unchanged", async () => {
    const { tenant, api } = await newTenant("p110-inbox-noop")
    const [inbox] = await addInboxes(tenant.workspaceId, 1)
    await api.patch(`/v1/inboxes/${inbox?.id}`, { markReadOnOutbound: true })
    const response = await api.patch<Inbox>(`/v1/inboxes/${inbox?.id}`, {})
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ id: inbox?.id, markReadOnOutbound: true })
  })

  it("updates a disconnected inbox too (it is not listed)", async () => {
    const { tenant, api } = await newTenant("p110-inbox-disc")
    const { inboxId } = await (await ext()).createInbox({
      workspaceId: tenant.workspaceId,
      channel: "webchat",
      name: "Old",
      sourceId: `src-${uniqueId()}`,
      status: "disconnected",
    })
    const response = await api.patch<Inbox>(`/v1/inboxes/${inboxId}`, { markReadOnOutbound: true })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ id: inboxId, status: "disconnected", markReadOnOutbound: true })
  })

  it("404 notFound for an unknown or foreign inbox, with or without a change", async () => {
    const { api } = await newTenant("p110-inbox-404")
    const { tenant: other } = await newTenant("p110-inbox-404-o")
    const [foreign] = await addInboxes(other.workspaceId, 1)
    for (const id of [UNKNOWN_ID, foreign?.id as string]) {
      expectNotFound(await api.patch(`/v1/inboxes/${id}`, { markReadOnOutbound: true }), "Inbox not found")
      expectNotFound(await api.patch(`/v1/inboxes/${id}`, {}), "Inbox not found")
    }
  })

  it("validates the input (422 invalidRequestData)", async () => {
    const { tenant, api } = await newTenant("p110-inbox-upd-invalid")
    const [inbox] = await addInboxes(tenant.workspaceId, 1)
    expectInvalid(await api.patch("/v1/inboxes/abc", { markReadOnOutbound: true }), "id")
    expectInvalid(await api.patch(`/v1/inboxes/${inbox?.id}`, { markReadOnOutbound: "yes" }), "markReadOnOutbound")
  })
})
