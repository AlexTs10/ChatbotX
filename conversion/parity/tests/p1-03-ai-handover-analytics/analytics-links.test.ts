// Magic link and ref link analytics: magicLinkStats, magicLinkContacts,
// refLinkStats, refLinkContacts. The links are created through the public
// API (magicLinks.create, reflinks.create); click rows are fixtures.
//
// Stats: clicks per local day of `timezone`, no zero-fill, no link check.
// Contacts: one row per contact inbox (first click in the range), newest
// first; `total` counts distinct contact inboxes. Both accept the legacy
// `startDate` / `endDate` names.

import { describe, expect, it } from "vitest"
import type { CreatedContact } from "../../src/fixtures/ext/p1-03.ts"
import { uniqueId } from "../../src/unique.ts"
import { type Api, expectInvalid, ext, syntheticId, tenantApi, uniqueName } from "./helpers.ts"

const RANGE = { from: "2025-08-01T00:00:00Z", to: "2025-08-03T23:59:59Z" }

type Kind = "magic" | "ref"
const ROUTE: Record<Kind, string> = { magic: "magic-links", ref: "ref-links" }

const createLink = async (api: Api, kind: Kind): Promise<string> => {
  if (kind === "magic") {
    const response = await api.post<{ id: string }>("/v1/magic-links", { name: `promo-${uniqueId()}`, url: "https://shop.parity.test/sale" })
    expect(response.status).toBe(201)
    return response.json.id
  }
  const flow = await api.post<{ id: string }>("/v1/flows", { name: uniqueName("Ref flow"), folderId: null })
  expect(flow.status).toBe(201)
  const response = await api.post<{ id: string }>("/v1/ref-links", { name: `ref${uniqueId()}`, flowId: flow.json.id, customFieldId: null })
  expect(response.status).toBe(201)
  return response.json.id
}

/**
 * K1 clicks twice on 08-01 (and once before the range), K2 late on 08-01
 * (08-02 in Ho Chi Minh City), K3 on 08-03, K4 only before the range, and
 * one click of a contact inbox that no longer exists.
 */
const seed = async (kind: Kind, label: string) => {
  const { tenant, api } = await tenantApi(label)
  const ws = tenant.workspaceId
  const linkId = await createLink(api, kind)
  const fx = await ext()
  const { inboxId } = await fx.createInbox({ workspaceId: ws, name: uniqueName("Inbox"), channel: "messenger" })
  const k: CreatedContact[] = []
  for (let i = 0; i < 4; i += 1) {
    k.push(await fx.createContact({ workspaceId: ws, inboxId, channel: "messenger", firstName: `L${i}` }))
  }
  const gone = { contactId: syntheticId(), contactInboxId: syntheticId() }
  const click = (who: { contactId: string; contactInboxId: string }, occurredAt: string) => ({
    workspaceId: ws,
    linkId,
    contactId: who.contactId,
    contactInboxId: who.contactInboxId,
    occurredAt,
  })
  const [k1, k2, k3, k4] = k as [CreatedContact, CreatedContact, CreatedContact, CreatedContact]
  await fx.insertLinkStats(kind, [
    click(k1, "2025-07-31T23:59:59Z"),
    click(k1, "2025-08-01T10:00:00Z"),
    click(k1, "2025-08-01T12:00:00Z"),
    click(k2, "2025-08-01T23:30:00Z"),
    click(gone, "2025-08-02T00:00:00Z"),
    click(k3, "2025-08-03T00:30:00Z"),
    click(k4, "2025-07-20T00:00:00Z"),
  ])
  return { tenant, api, linkId, k1, k2, k3, k4, gone }
}

const contactRow = (c: CreatedContact, occurredAt: string) => ({
  contactId: c.contactId,
  contactInboxId: c.contactInboxId,
  sourceId: c.sourceId,
  channel: "messenger",
  conversationId: c.conversationId,
  occurredAt,
})
/** A click whose contact inbox is gone: the row keeps only its time. */
const goneRow = (occurredAt: string) => ({ contactId: "", contactInboxId: "", sourceId: null, channel: null, conversationId: "", occurredAt })

describe.each(["magic", "ref"] as const)("%s link analytics", (kind) => {
  const route = ROUTE[kind]
  const opStats = kind === "magic" ? "analytics.magicLinkStats" : "analytics.refLinkStats"
  const opContacts = kind === "magic" ? "analytics.magicLinkContacts" : "analytics.refLinkContacts"

  it(`${opStats}: clicks per day in UTC and in a local timezone`, async () => {
    const { api, linkId } = await seed(kind, `an-${kind}-stats`)
    const utc = await api.get(`/v1/analytics/${route}/stats`, { query: { ...RANGE, linkId, timezone: "UTC" } })
    expect(utc.status).toBe(200)
    expect(utc.json).toEqual({
      data: [
        { dateReport: "2025-08-01", count: 3 },
        { dateReport: "2025-08-02", count: 1 },
        { dateReport: "2025-08-03", count: 1 },
      ],
    })
    const local = await api.get(`/v1/analytics/${route}/stats`, { query: { ...RANGE, linkId, timezone: "Asia/Ho_Chi_Minh" } })
    expect(local.json).toEqual({
      data: [
        { dateReport: "2025-08-01", count: 2 },
        { dateReport: "2025-08-02", count: 2 },
        { dateReport: "2025-08-03", count: 1 },
      ],
    })
    // Legacy parameter names.
    const legacy = await api.get(`/v1/analytics/${route}/stats`, {
      query: { startDate: RANGE.from, endDate: RANGE.to, linkId, timezone: "UTC" },
    })
    expect(legacy.json).toEqual(utc.json)
  })

  it(`${opStats}: is empty for an unknown or foreign link`, async () => {
    const { linkId } = await seed(kind, `an-${kind}-stats-foreign`)
    const other = await tenantApi(`an-${kind}-stats-other`)
    expect((await other.api.get(`/v1/analytics/${route}/stats`, { query: { ...RANGE, linkId, timezone: "UTC" } })).json).toEqual({ data: [] })
    expect((await other.api.get(`/v1/analytics/${route}/stats`, { query: { ...RANGE, linkId: "1", timezone: "UTC" } })).json).toEqual({ data: [] })
  })

  it(`${opStats}: rejects missing input (422)`, async () => {
    const { api } = await tenantApi(`an-${kind}-stats-bad`)
    expectInvalid(await api.get(`/v1/analytics/${route}/stats`, { query: { ...RANGE, timezone: "UTC" } }), "linkId")
    expectInvalid(await api.get(`/v1/analytics/${route}/stats`, { query: { ...RANGE, linkId: "1" } }), "timezone")
    expectInvalid(await api.get(`/v1/analytics/${route}/stats`, { query: { to: RANGE.to, linkId: "1", timezone: "UTC" } }), "from")
  })

  it(`${opContacts}: lists the contact inboxes newest first within the range`, async () => {
    const { api, linkId, k1, k2, k3 } = await seed(kind, `an-${kind}-contacts`)
    const response = await api.get(`/v1/analytics/${route}/contacts`, { query: { ...RANGE, linkId } })
    expect(response.status).toBe(200)
    expect(response.json).toEqual({
      data: [
        contactRow(k3, "2025-08-03T00:30:00.000Z"),
        goneRow("2025-08-02T00:00:00.000Z"),
        contactRow(k2, "2025-08-01T23:30:00.000Z"),
        contactRow(k1, "2025-08-01T10:00:00.000Z"),
      ],
      total: 4,
      page: 1,
      pageCount: 1,
    })
    const legacy = await api.get(`/v1/analytics/${route}/contacts`, { query: { startDate: RANGE.from, endDate: RANGE.to, linkId } })
    expect(legacy.json).toEqual(response.json)
  })

  it(`${opContacts}: without a full range lists every click, and pages`, async () => {
    const { api, linkId, k1, k2, k3, k4 } = await seed(kind, `an-${kind}-contacts-all`)
    const all = await api.get(`/v1/analytics/${route}/contacts`, { query: { linkId, from: RANGE.from } })
    expect(all.json).toEqual({
      data: [
        contactRow(k3, "2025-08-03T00:30:00.000Z"),
        goneRow("2025-08-02T00:00:00.000Z"),
        contactRow(k2, "2025-08-01T23:30:00.000Z"),
        contactRow(k1, "2025-07-31T23:59:59.000Z"),
        contactRow(k4, "2025-07-20T00:00:00.000Z"),
      ],
      total: 5,
      page: 1,
      pageCount: 1,
    })
    const page2 = await api.get(`/v1/analytics/${route}/contacts`, { query: { ...RANGE, linkId, perPage: 3, page: 2 } })
    expect(page2.json).toEqual({ data: [contactRow(k1, "2025-08-01T10:00:00.000Z")], total: 4, page: 2, pageCount: 2 })
    const beyond = await api.get(`/v1/analytics/${route}/contacts`, { query: { ...RANGE, linkId, perPage: 3, page: 5 } })
    expect(beyond.json).toEqual({ data: [], total: 4, page: 5, pageCount: 2 })
  })

  it(`${opContacts}: is an empty first page for an unknown or foreign link`, async () => {
    const { linkId } = await seed(kind, `an-${kind}-contacts-foreign`)
    const other = await tenantApi(`an-${kind}-contacts-other`)
    const empty = { data: [], total: 0, page: 1, pageCount: 0 }
    expect((await other.api.get(`/v1/analytics/${route}/contacts`, { query: { ...RANGE, linkId, page: 3 } })).json).toEqual(empty)
    expect((await other.api.get(`/v1/analytics/${route}/contacts`, { query: { linkId: "1" } })).json).toEqual(empty)
  })

  it(`${opContacts}: rejects bad input (422)`, async () => {
    const { api } = await tenantApi(`an-${kind}-contacts-bad`)
    expectInvalid(await api.get(`/v1/analytics/${route}/contacts`, { query: { ...RANGE } }), "linkId")
    expectInvalid(await api.get(`/v1/analytics/${route}/contacts`, { query: { linkId: "1", perPage: 51 } }), "perPage")
    expectInvalid(await api.get(`/v1/analytics/${route}/contacts`, { query: { linkId: "1", page: 0 } }), "page")
  })
})
