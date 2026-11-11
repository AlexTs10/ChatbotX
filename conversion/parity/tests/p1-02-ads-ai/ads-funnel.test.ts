// DB-only ad reports: ads.getFunnel, ads.getFunnelTimeseries,
// ads.getCapiDelivery, ads.listConversionExportRows, ads.getConversion.
// Data is seeded with the fixture extension (ad-referred contact inboxes and
// conversion events on one Messenger page), in October 2026.

import { describe, expect, it } from "vitest"
import { createTenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { Normalizer, unordered } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import { adsFixtures, expectError, expectValidationIssue, MISSING_ID, uniqueDigits } from "./ads-helpers.ts"

const RANGE = { since: "2026-10-01T00:00:00.000Z", until: "2026-10-07T23:59:59.999Z" }

/** Seeds one workspace: 5 contacts (4 ad-referred) and 5 conversion events. */
const seed = async (label: string) => {
  const tenant = await createTenant({ label })
  const ext = await adsFixtures()
  const pageId = uniqueDigits()
  const page = await ext.createMessengerChannel({ workspaceId: tenant.workspaceId, pageId, name: `Page ${pageId}` })
  const contact = (n: number, referral: Record<string, unknown> | undefined, firstInteractionAt: string) =>
    ext.createContactInbox({
      workspaceId: tenant.workspaceId,
      inboxId: page.inboxId,
      channel: "messenger",
      sourceId: `psid-${n}-${uniqueId()}`,
      firstName: `First${n}`,
      lastName: `Last${n}`,
      email: `contact${n}-${uniqueId()}@parity.test`,
      phoneNumber: `+1555000000${n}`,
      firstInteractionAt,
      referral,
    })
  const ad1 = { adId: "120001", source: "ADS", type: "OPEN_THREAD" }
  const ad2 = { adId: "120002", source: "ADS", type: "OPEN_THREAD" }
  const c1 = await contact(1, ad1, "2026-10-02T10:00:00.000Z")
  const c2 = await contact(2, ad1, "2026-10-03T12:00:00.000Z")
  const c3 = await contact(3, ad2, "2026-10-03T13:00:00.000Z")
  const c4 = await contact(4, ad1, "2026-09-20T10:00:00.000Z") // before the range
  const c5 = await contact(5, undefined, "2026-10-04T10:00:00.000Z") // organic
  const event = (input: Parameters<typeof ext.createConversionEvent>[0]) => ext.createConversionEvent(input)
  const base = { workspaceId: tenant.workspaceId, channel: "messenger" as const, integrationId: page.integrationId }
  const e1 = await event({
    ...base,
    eventType: "lead",
    occurredAt: "2026-10-02T11:00:00.000Z",
    sourceEventId: `e1-${uniqueId()}`,
    adId: "120001",
    contactInboxId: c1.contactInboxId,
    capiStatus: "sent",
  })
  const e2 = await event({
    ...base,
    eventType: "purchase",
    occurredAt: "2026-10-03T14:00:00.000Z",
    sourceEventId: `e2-${uniqueId()}`,
    adId: "120001",
    contactInboxId: c2.contactInboxId,
    currency: "USD",
    value: "25.50",
    orderId: "ORD-1",
    contents: [{ id: "sku-1", quantity: 2, itemPrice: 12.75 }],
    capiStatus: "pending",
  })
  const e3 = await event({
    ...base,
    eventType: "purchase",
    occurredAt: "2026-10-04T09:00:00.000Z",
    sourceEventId: `e3-${uniqueId()}`,
    adId: "120002",
    contactInboxId: c3.contactInboxId,
    currency: "USD",
    value: "10",
    capiStatus: "failed",
  })
  const e4 = await event({
    ...base,
    eventType: "lead",
    occurredAt: "2026-10-05T20:00:00.000Z",
    sourceEventId: `e4-${uniqueId()}`,
    adId: "120002",
    contactInboxId: c3.contactInboxId,
    capiStatus: "skipped_no_scope",
  })
  const e5 = await event({
    ...base,
    eventType: "lead",
    occurredAt: "2026-09-25T10:00:00.000Z", // before the range
    sourceEventId: `e5-${uniqueId()}`,
    adId: "120001",
    contactInboxId: c4.contactInboxId,
    capiStatus: "skipped_region",
  })
  return { tenant, api: apiClient(tenant.token), page, contacts: { c1, c2, c3, c4, c5 }, events: { e1, e2, e3, e4, e5 } }
}

const funnelRows = (rows: Array<{ adId: string | null }>) => unordered(rows, (row) => row.adId ?? "")

describe("ads.getFunnel (GET /v1/ads/funnel)", () => {
  it("counts ad conversations, leads, purchases and revenue per ad", async () => {
    const { api, page } = await seed("funnel")
    const response = await api.get<{ totals: unknown; perAd: Array<{ adId: string | null }> }>("/v1/ads/funnel", {
      query: { ...RANGE, channel: "messenger" },
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json.totals).toEqual({ conversations: 3, leads: 2, purchases: 2, revenue: 35.5 })
    expect(funnelRows(response.json.perAd)).toEqual([
      { adId: "120001", adName: null, conversations: 2, leads: 1, purchases: 1, revenue: 25.5 },
      { adId: "120002", adName: null, conversations: 1, leads: 1, purchases: 1, revenue: 10 },
    ])

    const scoped = await api.get("/v1/ads/funnel", {
      query: { ...RANGE, channel: "messenger", integrationMessengerId: page.integrationId },
    })
    expect(scoped.json).toMatchObject({ totals: { conversations: 3, leads: 2, purchases: 2, revenue: 35.5 } })
  })

  it("adds channels under allChannels and defaults to whatsapp when channel is omitted", async () => {
    const { api } = await seed("funnel-all")
    const all = await api.get<{ totals: unknown; perAd: Array<{ adId: string | null }> }>("/v1/ads/funnel", {
      query: { ...RANGE, allChannels: true },
    })
    expect(all.status, all.text).toBe(200)
    expect(all.json.totals).toEqual({ conversations: 3, leads: 2, purchases: 2, revenue: 35.5 })
    expect(funnelRows(all.json.perAd)).toEqual([
      { adId: "120001", adName: null, conversations: 2, leads: 1, purchases: 1, revenue: 25.5, channels: ["messenger"] },
      { adId: "120002", adName: null, conversations: 1, leads: 1, purchases: 1, revenue: 10, channels: ["messenger"] },
    ])

    const whatsappDefault = await api.get("/v1/ads/funnel", { query: RANGE })
    expect(whatsappDefault.json).toEqual({ totals: { conversations: 0, leads: 0, purchases: 0, revenue: 0 }, perAd: [] })
    const facebook = await api.get("/v1/ads/funnel", { query: { ...RANGE, channel: "facebook" } })
    expect(facebook.json).toEqual({ totals: { conversations: 0, leads: 0, purchases: 0, revenue: 0 }, perAd: [] })
  })

  it("rejects bad ranges and filter combinations with 422", async () => {
    const tenant = await createTenant({ label: "funnel-422" })
    const api = apiClient(tenant.token)
    expectValidationIssue(await api.get("/v1/ads/funnel", { query: { since: RANGE.until, until: RANGE.since } }), ["until"])
    expectValidationIssue(
      await api.get("/v1/ads/funnel", { query: { since: "2025-01-01T00:00:00Z", until: "2026-01-02T00:00:00Z" } }),
      ["until"],
    )
    expectValidationIssue(await api.get("/v1/ads/funnel", { query: { ...RANGE, allChannels: true, channel: "messenger" } }), [
      "allChannels",
    ])
    expectValidationIssue(
      await api.get("/v1/ads/funnel", {
        query: { ...RANGE, integrationWhatsappId: "1000000000000000001", integrationMessengerId: "1000000000000000002" },
      }),
      ["integrationWhatsappId"],
    )
    expectValidationIssue(await api.get("/v1/ads/funnel", { query: { until: RANGE.until } }), ["since"])
    expectValidationIssue(await api.get("/v1/ads/funnel", { query: { ...RANGE, since: "not-a-date" } }), ["since"])
    expectValidationIssue(await api.get("/v1/ads/funnel", { query: { ...RANGE, channel: "tiktok" } }), ["channel"])
  })
})

describe("ads.getFunnelTimeseries (GET /v1/ads/funnel/timeseries)", () => {
  it("buckets per day and ad, in the given time zone", async () => {
    const { api } = await seed("funnel-ts")
    type Row = { date: string; adId: string | null }
    const sortRows = (rows: Row[]) => unordered(rows, (row) => `${row.date}:${row.adId}`)
    const utc = await api.get<{ data: Row[] }>("/v1/ads/funnel/timeseries", { query: { ...RANGE, channel: "messenger" } })
    expect(utc.status, utc.text).toBe(200)
    expect(sortRows(utc.json.data)).toEqual([
      { date: "2026-10-02", adId: "120001", conversations: 1, leads: 1, purchases: 0 },
      { date: "2026-10-03", adId: "120001", conversations: 1, leads: 0, purchases: 1 },
      { date: "2026-10-03", adId: "120002", conversations: 1, leads: 0, purchases: 0 },
      { date: "2026-10-04", adId: "120002", conversations: 0, leads: 0, purchases: 1 },
      { date: "2026-10-05", adId: "120002", conversations: 0, leads: 1, purchases: 0 },
    ])
    const tokyo = await api.get<{ data: Row[] }>("/v1/ads/funnel/timeseries", {
      query: { ...RANGE, channel: "messenger", timezone: "Asia/Tokyo" },
    })
    expect(sortRows(tokyo.json.data).at(-1)).toEqual({
      date: "2026-10-06",
      adId: "120002",
      conversations: 0,
      leads: 1,
      purchases: 0,
    })
    expectValidationIssue(
      await api.get("/v1/ads/funnel/timeseries", { query: { since: RANGE.until, until: RANGE.since } }),
      ["until"],
    )
  })
})

describe("ads.getCapiDelivery (GET /v1/ads/capi-delivery)", () => {
  it("counts the range's events by Conversions API status", async () => {
    const { api } = await seed("capi")
    const response = await api.get("/v1/ads/capi-delivery", { query: { ...RANGE, channel: "messenger" } })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ sent: 1, pending: 1, failed: 1, skippedNoScope: 1, skippedRegion: 0 })
    const wide = await api.get("/v1/ads/capi-delivery", {
      query: { since: "2026-09-01T00:00:00Z", until: "2026-10-31T00:00:00Z", channel: "messenger" },
    })
    expect(wide.json).toEqual({ sent: 1, pending: 1, failed: 1, skippedNoScope: 1, skippedRegion: 1 })
    const whatsapp = await api.get("/v1/ads/capi-delivery", { query: RANGE })
    expect(whatsapp.json).toEqual({ sent: 0, pending: 0, failed: 0, skippedNoScope: 0, skippedRegion: 0 })
    expectValidationIssue(await api.get("/v1/ads/capi-delivery", { query: { ...RANGE, allChannels: true, channel: "messenger" } }), [
      "allChannels",
    ])
  })
})

describe("ads.listConversionExportRows (GET /v1/ads/conversions/export)", () => {
  it("returns contact-level rows per segment with cursor paging", async () => {
    const { api, contacts, events } = await seed("export")
    const query = { ...RANGE, channel: "messenger", segment: "leads" }
    const leads = await api.get<{ data: Array<Record<string, unknown>>; nextAfterId: string | null }>(
      "/v1/ads/conversions/export",
      { query },
    )
    expect(leads.status, leads.text).toBe(200)
    expect(leads.json.data).toEqual([
      {
        id: events.e1.eventId,
        contactId: contacts.c1.contactId,
        contactName: "First1 Last1",
        phoneNumber: "+15550000001",
        email: expect.stringMatching(/^contact1-.*@parity\.test$/),
        adId: "120001",
        occurredAt: "2026-10-02T11:00:00.000Z",
      },
      {
        id: events.e4.eventId,
        contactId: contacts.c3.contactId,
        contactName: "First3 Last3",
        phoneNumber: "+15550000003",
        email: expect.stringMatching(/^contact3-/),
        adId: "120002",
        occurredAt: "2026-10-05T20:00:00.000Z",
      },
    ])
    expect(leads.json.nextAfterId).toBeNull()

    const page1 = await api.get<{ data: Array<{ id: string }>; nextAfterId: string | null }>("/v1/ads/conversions/export", {
      query: { ...query, limit: 1 },
    })
    expect(page1.json.data.map((row) => row.id)).toEqual([events.e1.eventId])
    expect(page1.json.nextAfterId).toBe(events.e1.eventId)
    const page2 = await api.get<{ data: Array<{ id: string }>; nextAfterId: string | null }>("/v1/ads/conversions/export", {
      query: { ...query, limit: 1, afterId: events.e1.eventId },
    })
    expect(page2.json.data.map((row) => row.id)).toEqual([events.e4.eventId])
    expect(page2.json.nextAfterId).toBeNull()

    const conversations = await api.get<{ data: Array<Record<string, unknown>> }>("/v1/ads/conversions/export", {
      query: { ...RANGE, channel: "messenger", segment: "conversations" },
    })
    expect(conversations.json.data.map((row) => [row.id, row.contactId, row.adId, row.occurredAt])).toEqual([
      [contacts.c1.contactInboxId, contacts.c1.contactId, "120001", "2026-10-02T10:00:00.000Z"],
      [contacts.c2.contactInboxId, contacts.c2.contactId, "120001", "2026-10-03T12:00:00.000Z"],
      [contacts.c3.contactInboxId, contacts.c3.contactId, "120002", "2026-10-03T13:00:00.000Z"],
    ])
    const byAd = await api.get<{ data: Array<{ id: string }> }>("/v1/ads/conversions/export", {
      query: { ...RANGE, channel: "messenger", segment: "conversations", adId: "120002" },
    })
    expect(byAd.json.data.map((row) => row.id)).toEqual([contacts.c3.contactInboxId])

    const allPurchases = await api.get<{ data: Array<Record<string, unknown>> }>("/v1/ads/conversions/export", {
      query: { ...RANGE, allChannels: true, segment: "purchases" },
    })
    expect(allPurchases.json.data.map((row) => [row.id, row.channel])).toEqual([
      [events.e2.eventId, "messenger"],
      [events.e3.eventId, "messenger"],
    ])
    const whatsappDefault = await api.get("/v1/ads/conversions/export", { query: { ...RANGE, segment: "purchases" } })
    expect(whatsappDefault.json).toEqual({ data: [], nextAfterId: null })
  })

  it("rejects bad segments, limits and ranges with 422", async () => {
    const tenant = await createTenant({ label: "export-422" })
    const api = apiClient(tenant.token)
    expectValidationIssue(await api.get("/v1/ads/conversions/export", { query: { ...RANGE, segment: "clicks" } }), ["segment"])
    expectValidationIssue(await api.get("/v1/ads/conversions/export", { query: RANGE }), ["segment"])
    expectValidationIssue(await api.get("/v1/ads/conversions/export", { query: { ...RANGE, segment: "leads", limit: 1001 } }), [
      "limit",
    ])
    expectValidationIssue(await api.get("/v1/ads/conversions/export", { query: { ...RANGE, segment: "leads", limit: 0 } }), [
      "limit",
    ])
    expectValidationIssue(
      await api.get("/v1/ads/conversions/export", { query: { segment: "leads", since: RANGE.until, until: RANGE.since } }),
      ["until"],
    )
    expectValidationIssue(
      await api.get("/v1/ads/conversions/export", { query: { ...RANGE, segment: "leads", afterId: "abc" } }),
      ["afterId"],
    )
  })
})

describe("ads.getConversion (GET /v1/ads/conversions/{id})", () => {
  it("returns one event without workspaceId or referralEventId; 404 otherwise", async () => {
    const normalizer = new Normalizer()
    const { api, page, contacts, events } = await seed("conversion")
    const response = await api.get<Record<string, unknown>>(`/v1/ads/conversions/${events.e2.eventId}`)
    expect(response.status, response.text).toBe(200)
    expect(
      normalizer.normalize(response.json, {
        keep: ["occurredAt"],
        replace: { [page.integrationId]: "<page>", [contacts.c2.contactInboxId]: "<contactInbox>" },
      }),
    ).toEqual({
      id: "<id:1>",
      channel: "messenger",
      integrationWhatsappId: null,
      integrationMessengerId: "<page>",
      integrationInstagramId: null,
      wabaId: null,
      source: "rule",
      eventType: "purchase",
      ctwaClid: null,
      adId: "120001",
      contactInboxId: "<contactInbox>",
      currency: "USD",
      value: "25.50",
      orderId: "ORD-1",
      contents: [{ id: "sku-1", quantity: 2, itemPrice: 12.75 }],
      occurredAt: "2026-10-03T14:00:00.000Z",
      sourceEventId: expect.stringMatching(/^e2-/),
      capiStatus: "pending",
      capiSentAt: null,
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })
    expectError(await api.get(`/v1/ads/conversions/${MISSING_ID}`), 404, "notFound", "Ads conversion event not found")
    const other = await createTenant({ label: "conversion-other" })
    expectError(await apiClient(other.token).get(`/v1/ads/conversions/${events.e2.eventId}`), 404, "notFound")
    expectValidationIssue(await api.get("/v1/ads/conversions/abc"), ["id"])
  })
})
