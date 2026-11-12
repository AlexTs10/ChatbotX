// reflinks.list / get / create / update / delete / getChatWidget /
// updateChatWidget. A ref link carries one chat link per connected,
// linkable inbox (here a webchat inbox: no third-party call).

import { describe, expect, it } from "vitest"
import { uniqueId } from "../../src/unique.ts"
import {
  type Api,
  createPublishedFlow,
  createWebchat,
  expectInvalid,
  expectNotFound,
  expectValidation,
  ISO,
  newTenant,
  SNOWFLAKE,
  sortQuery,
  UNKNOWN_ID,
  uniqueName,
  uploadMediaFile,
} from "./helpers.ts"

type Link = { inboxId: string; inboxName: string; channel: string; url: string; receivesRef: boolean }
type Reflink = Record<string, unknown> & { id: string; name: string; flowId: string; links: Link[] }
type Widget = {
  reflinkId: string
  authorizedDomains: string[]
  hiddenInboxIds: string[]
  logoFileId: string | null
  logoUrl: string | null
  logoBackgroundColor: string
  brandName: string | null
  brandUrl: string | null
  scriptUrl: string
  embedCode: string
  channels: Link[]
}

const refName = () => `Ref${uniqueId()}`.slice(0, 20)

const create = async (api: Api, body: Record<string, unknown>): Promise<Reflink> => {
  // `customFieldId` is required (nullable): send null unless the test sets it.
  const response = await api.post<Reflink>("/v1/ref-links", { customFieldId: null, ...body })
  expect(response.status, response.text).toBe(201)
  return response.json
}

const expectWebchatLink = (link: Link | undefined, input: { workspaceId: string; inboxId: string; inboxName: string }) => {
  expect(link).toMatchObject({ inboxId: input.inboxId, inboxName: input.inboxName, channel: "webchat", receivesRef: true })
  const url = new URL(link?.url as string)
  expect(`${url.origin}${url.pathname}`).toBe("http://app/webchat")
  expect(url.searchParams.get("workspaceId")).toBe(input.workspaceId)
  expect(url.searchParams.get("webchatId")).toBeTruthy()
  expect(url.searchParams.get("ref")).toBeTruthy()
}

describe("reflinks.create / get", () => {
  it("creates a ref link with one link per connected inbox", async () => {
    const { tenant, api } = await newTenant("p114-rl-create")
    const flowId = await createPublishedFlow(api)
    const name = refName()
    const bare = await create(api, { name, flowId })
    expect(bare).toMatchObject({
      id: expect.stringMatching(SNOWFLAKE),
      name,
      type: "refLink",
      flowId,
      customFieldId: null,
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
      links: [],
    })

    const inboxName = uniqueName("Ref chat")
    const webchat = await createWebchat(api, inboxName)
    const got = await api.get<Reflink>(`/v1/ref-links/${bare.id}`)
    expect(got.status, got.text).toBe(200)
    expect(got.json).toMatchObject({ id: bare.id, name, flowId })
    expect(got.json.links).toHaveLength(1)
    expectWebchatLink(got.json.links[0], { workspaceId: tenant.workspaceId, inboxId: webchat.inboxId, inboxName })
    // Another ref link gets another ref value.
    const second = await create(api, { name: refName(), flowId, customFieldId: "" })
    expect(second.customFieldId).toBeNull()
    expect(new URL(second.links[0]?.url as string).searchParams.get("ref")).not.toBe(
      new URL(got.json.links[0]?.url as string).searchParams.get("ref"),
    )
  })

  it("422 for a taken name and bad input; 404 unknown id", async () => {
    const { api } = await newTenant("p114-rl-refused")
    const flowId = await createPublishedFlow(api)
    const name = refName()
    await create(api, { name, flowId })
    expectValidation(await api.post("/v1/ref-links", { name, flowId, customFieldId: null }), "Name is already taken")
    expectInvalid(await api.post("/v1/ref-links", { name: "with_underscore", flowId, customFieldId: null }), "name")
    expectInvalid(await api.post("/v1/ref-links", { name: "x".repeat(51), flowId, customFieldId: null }), "name")
    expectInvalid(await api.post("/v1/ref-links", { name: refName(), customFieldId: null }), "flowId")
    expectInvalid(await api.post("/v1/ref-links", { name: refName(), flowId }), "customFieldId")
    expectNotFound(await api.get(`/v1/ref-links/${UNKNOWN_ID}`), "Reflink not found")
  })
})

describe("reflinks.list", () => {
  it("is newest first by default, sorts by name, pages and filters by keyword", async () => {
    const { api } = await newTenant("p114-rl-list")
    const flowId = await createPublishedFlow(api)
    const token = uniqueId().slice(0, 10)
    const created: Reflink[] = []
    for (const prefix of ["B", "C", "A"]) {
      created.push(await create(api, { name: `${prefix}${token}`, flowId }))
    }
    const [b, c, a] = created as [Reflink, Reflink, Reflink]
    const page1 = await api.get<{ data: Reflink[]; pageCount: number }>("/v1/ref-links", { query: { perPage: 2 } })
    expect(page1.status, page1.text).toBe(200)
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data.map((reflink) => reflink.id)).toEqual([a.id, c.id])
    expect(page1.json.data[0]).toEqual(a)
    const page2 = await api.get<{ data: Reflink[] }>("/v1/ref-links", { query: { perPage: 2, page: 2 } })
    expect(page2.json.data.map((reflink) => reflink.id)).toEqual([b.id])
    const byName = await api.get<{ data: Reflink[] }>(`/v1/ref-links?${sortQuery("name", false)}`)
    expect(byName.json.data.map((reflink) => reflink.id)).toEqual([a.id, b.id, c.id])
    const filtered = await api.get<{ data: Reflink[] }>("/v1/ref-links", { query: { keyword: `c${token}`.toUpperCase() } })
    expect(filtered.json.data.map((reflink) => reflink.id)).toEqual([c.id])
    expectInvalid(await api.get(`/v1/ref-links?${sortQuery("flowId", false)}`), "sort.0.id")
  })
})

describe("reflinks.update / delete", () => {
  it("update changes the sent fields; 422 on a taken name; 404 unknown", async () => {
    const { api } = await newTenant("p114-rl-update")
    const flowId = await createPublishedFlow(api)
    const otherFlow = await createPublishedFlow(api)
    const reflink = await create(api, { name: refName(), flowId })
    const taken = await create(api, { name: refName(), flowId })
    const newName = refName()
    const response = await api.patch<Reflink>(`/v1/ref-links/${reflink.id}`, { name: newName, flowId: otherFlow })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ id: reflink.id, name: newName, flowId: otherFlow, links: [] })
    expectValidation(await api.patch(`/v1/ref-links/${reflink.id}`, { name: taken.name }), "Name is already taken")
    expectNotFound(await api.patch(`/v1/ref-links/${UNKNOWN_ID}`, { name: refName() }), "Reflink not found")
  })

  it("delete is 204; an unknown id is also 204", async () => {
    const { api } = await newTenant("p114-rl-delete")
    const flowId = await createPublishedFlow(api)
    const reflink = await create(api, { name: refName(), flowId })
    const response = await api.delete(`/v1/ref-links/${reflink.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectNotFound(await api.get(`/v1/ref-links/${reflink.id}`), "Reflink not found")
    expect((await api.delete(`/v1/ref-links/${UNKNOWN_ID}`)).status).toBe(204)
  })
})

describe("reflinks.getChatWidget / updateChatWidget", () => {
  it("returns the defaults, script URL and embed code; saves domains, hidden inboxes, logo and brand", async () => {
    const { tenant, api } = await newTenant("p114-rl-widget")
    const flowId = await createPublishedFlow(api)
    const reflink = await create(api, { name: refName(), flowId })
    const shownName = uniqueName("Shown chat")
    const shown = await createWebchat(api, shownName)
    const hidden = await createWebchat(api, uniqueName("Hidden chat"))
    const scriptUrl = "http://app/chat-widget/ref-widget.js"

    const initial = await api.get<Widget>(`/v1/ref-links/${reflink.id}/chat-widget`)
    expect(initial.status, initial.text).toBe(200)
    expect({ ...initial.json, channels: initial.json.channels.length }).toEqual({
      reflinkId: reflink.id,
      authorizedDomains: [],
      hiddenInboxIds: [],
      logoFileId: null,
      logoUrl: null,
      logoBackgroundColor: "#111827",
      brandName: null,
      brandUrl: null,
      scriptUrl,
      embedCode: `<script async src="${scriptUrl}" data-reflink-id="${reflink.id}"></script>`,
      channels: 2,
    })

    const logo = await uploadMediaFile(api)
    const response = await api.put<Widget>(`/v1/ref-links/${reflink.id}/chat-widget`, {
      authorizedDomains: ["shop.example.com", "example.org"],
      hiddenInboxIds: [hidden.inboxId, UNKNOWN_ID],
      logoFileId: logo.id,
      logoBackgroundColor: "#FF0000",
      brandName: "Parity Brand",
      brandUrl: "https://example.com",
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({
      reflinkId: reflink.id,
      authorizedDomains: ["shop.example.com", "example.org"],
      hiddenInboxIds: [hidden.inboxId],
      logoFileId: logo.id,
      logoUrl: `http://app/storage/${logo.path}`,
      logoBackgroundColor: "#FF0000",
      brandName: "Parity Brand",
      brandUrl: "https://example.com",
      scriptUrl,
    })
    expect(response.json.channels).toHaveLength(1)
    expectWebchatLink(response.json.channels[0], {
      workspaceId: tenant.workspaceId,
      inboxId: shown.inboxId,
      inboxName: shownName,
    })
    const again = await api.get<Widget>(`/v1/ref-links/${reflink.id}/chat-widget`)
    expect(again.json).toEqual(response.json)

    const cleared = await api.put<Widget>(`/v1/ref-links/${reflink.id}/chat-widget`, {
      authorizedDomains: [],
      hiddenInboxIds: [],
      logoFileId: "",
      logoBackgroundColor: "#111827",
      brandName: "",
      brandUrl: "",
    })
    expect(cleared.json).toMatchObject({ logoFileId: null, logoUrl: null, brandName: null, brandUrl: null, hiddenInboxIds: [] })
    expect(cleared.json.channels).toHaveLength(2)
  })

  it("rejects a brand name without URL, a bad color or domain, and a logo that is not a workspace image", async () => {
    const { api } = await newTenant("p114-rl-widget-invalid")
    const flowId = await createPublishedFlow(api)
    const reflink = await create(api, { name: refName(), flowId })
    const base = {
      authorizedDomains: [],
      hiddenInboxIds: [],
      logoFileId: "",
      logoBackgroundColor: "#111827",
      brandName: "",
      brandUrl: "",
    }
    const put = (body: Record<string, unknown>) => api.put(`/v1/ref-links/${reflink.id}/chat-widget`, { ...base, ...body })
    expectInvalid(await put({ brandName: "Only name" }), "brandUrl")
    expectInvalid(await put({ brandUrl: "https://example.com" }), "brandName")
    expectInvalid(await put({ logoBackgroundColor: "red" }), "logoBackgroundColor")
    expectInvalid(await put({ authorizedDomains: ["not a host"] }), "authorizedDomains.0")
    expectValidation(await put({ logoFileId: UNKNOWN_ID }), "Logo must be an image")
    expectNotFound(await api.put(`/v1/ref-links/${UNKNOWN_ID}/chat-widget`, base), "Reflink not found")
    expectNotFound(await api.get(`/v1/ref-links/${UNKNOWN_ID}/chat-widget`), "Reflink not found")
  })
})
