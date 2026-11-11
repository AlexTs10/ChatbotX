// Contact analytics: contactCountsPerDay, newContactCountsPerDay,
// blockedContactsPerDay, blockedContactsCount, newContactsCount,
// contactsCount, contactsByDimension, activeContactsCount,
// macActiveContactCount.
//
// Events are seeded with fixed 2025 times, so every read takes the raw
// hypertable path (the hourly caggs are only read for ranges that start in
// the last 6 days, and their refresh lag is not deterministic). Day buckets
// are local days of `timezone`; series dates are the local day at 00:00 UTC.

import { beforeAll, describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import { uniqueId } from "../../src/unique.ts"
import { type Api, day, expectInvalid, ext, qs, syntheticId, tenantApi, uniqueName } from "./helpers.ts"

const UTC_RANGE = { from: "2025-03-01T00:00:00Z", to: "2025-03-05T23:59:59Z" }
const HCM_RANGE = { from: "2025-03-01T00:00:00+07:00", to: "2025-03-05T23:59:59+07:00", timezone: "Asia/Ho_Chi_Minh" }
const MONTH_RANGE = { from: "2025-01-01T00:00:00Z", to: "2025-03-31T23:59:59Z" }

const series = (days: string[], counts: number[]) => days.map((key, index) => ({ date: day(key), count: counts[index] }))
const MARCH_1_TO_5 = ["2025-03-01", "2025-03-02", "2025-03-03", "2025-03-04", "2025-03-05"]
const Q1_MONTHS = ["2025-01-01", "2025-02-01", "2025-03-01"]

/**
 * One workspace for the contact series tests (only the active-contacts test
 * adds rows, to a table no other test reads):
 * - inbox I1 (messenger) with 2 contacts, I2 (telegram) with 1 contact
 *   (InboxContactStat 2 / 1);
 * - contact events C0..C3, see the inline comments.
 */
describe("contact analytics over seeded events", () => {
  let tenant: Tenant
  let api: Api
  let i1: string
  let i2: string

  beforeAll(async () => {
    ;({ tenant, api } = await tenantApi("an-contacts"))
    const fx = await ext()
    const ws = tenant.workspaceId
    i1 = (await fx.createInbox({ workspaceId: ws, name: uniqueName("I1"), channel: "messenger" })).inboxId
    i2 = (await fx.createInbox({ workspaceId: ws, name: uniqueName("I2"), channel: "telegram" })).inboxId
    await fx.createContact({ workspaceId: ws, inboxId: i1, channel: "messenger" })
    await fx.createContact({ workspaceId: ws, inboxId: i1, channel: "messenger" })
    await fx.createContact({ workspaceId: ws, inboxId: i2, channel: "telegram" })
    const [c0, c1, c2, c3] = [syntheticId(), syntheticId(), syntheticId(), syntheticId()] as [string, string, string, string]
    const m = { channel: "messenger", inboxId: i1 }
    const t = { channel: "telegram", inboxId: i2 }
    const vi = { country: "VN", language: "vi" }
    await fx.insertContactEvents([
      // before every range: the running-total baseline
      { workspaceId: ws, contactId: c0, eventType: "contact_created", occurredAt: "2025-02-20T00:00:00Z", source: "direct", ...m, ...vi },
      { workspaceId: ws, contactId: c1, eventType: "contact_created", occurredAt: "2025-03-02T10:00:00Z", source: "direct", ...m, ...vi },
      // duplicate created event of C1 on the same day
      { workspaceId: ws, contactId: c1, eventType: "contact_created", occurredAt: "2025-03-02T12:00:00Z", source: "direct", ...m, ...vi },
      // 03-02 in UTC, 03-03 in Ho Chi Minh City (+07:00)
      { workspaceId: ws, contactId: c2, eventType: "contact_created", occurredAt: "2025-03-02T23:30:00Z", source: "ads", ...m, ...vi },
      { workspaceId: ws, contactId: c3, eventType: "contact_created", occurredAt: "2025-03-04T05:00:00Z", source: "direct", ...t, country: "US", language: "en" },
      { workspaceId: ws, contactId: c3, eventType: "contact_deleted", occurredAt: "2025-03-05T08:00:00Z", source: "direct", ...t },
      { workspaceId: ws, contactId: c1, eventType: "contact_blocked", occurredAt: "2025-03-03T01:00:00Z", ...m },
      { workspaceId: ws, contactId: c1, eventType: "contact_blocked", occurredAt: "2025-03-03T02:00:00Z", ...m },
      { workspaceId: ws, contactId: c2, eventType: "contact_blocked", occurredAt: "2025-03-04T16:30:00Z", ...m },
    ])
  })

  describe("analytics.contactCountsPerDay", () => {
    it("charts the running total per UTC day from the baseline", async () => {
      const response = await api.get("/v1/analytics/contact-counts-per-day", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({ data: series(MARCH_1_TO_5, [1, 4, 4, 5, 4]) })
    })

    it("buckets by local day of the timezone", async () => {
      const response = await api.get("/v1/analytics/contact-counts-per-day", { query: HCM_RANGE })
      expect(response.json).toEqual({ data: series(MARCH_1_TO_5, [1, 3, 4, 5, 4]) })
    })

    it("uses monthly buckets past 60 days", async () => {
      const response = await api.get("/v1/analytics/contact-counts-per-day", { query: MONTH_RANGE })
      expect(response.json).toEqual({ data: series(Q1_MONTHS, [0, 1, 4]) })
    })

    it("anchors an inbox-filtered total on the current per-inbox count", async () => {
      const response = await api.get(`/v1/analytics/contact-counts-per-day${qs({ ...UTC_RANGE, inboxIds: [i1] })}`)
      expect(response.json).toEqual({ data: series(MARCH_1_TO_5, [0, 3, 3, 3, 3]) })
    })
  })

  describe("analytics.newContactCountsPerDay", () => {
    it("counts distinct new contacts per UTC day", async () => {
      const response = await api.get("/v1/analytics/new-contact-counts-per-day", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({ data: series(MARCH_1_TO_5, [0, 2, 0, 1, 0]) })
    })

    it("buckets by local day of the timezone", async () => {
      const response = await api.get("/v1/analytics/new-contact-counts-per-day", { query: HCM_RANGE })
      expect(response.json).toEqual({ data: series(MARCH_1_TO_5, [0, 1, 1, 1, 0]) })
    })

    it("uses monthly buckets past 60 days", async () => {
      const response = await api.get("/v1/analytics/new-contact-counts-per-day", { query: MONTH_RANGE })
      expect(response.json).toEqual({ data: series(Q1_MONTHS, [0, 1, 3]) })
    })

    it("narrows to a channel", async () => {
      const response = await api.get("/v1/analytics/new-contact-counts-per-day", { query: { ...UTC_RANGE, channels: "telegram" } })
      expect(response.json).toEqual({ data: series(MARCH_1_TO_5, [0, 0, 0, 1, 0]) })
    })
  })

  describe("analytics.blockedContactsPerDay", () => {
    it("counts distinct blocked contacts per day", async () => {
      const response = await api.get("/v1/analytics/blocked-contacts-per-day", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({ data: series(MARCH_1_TO_5, [0, 0, 1, 1, 0]) })
      expect((await api.get("/v1/analytics/blocked-contacts-per-day", { query: HCM_RANGE })).json).toEqual({
        data: series(MARCH_1_TO_5, [0, 0, 1, 1, 0]),
      })
    })

    it("uses monthly buckets past 60 days and honours the inbox filter", async () => {
      expect((await api.get("/v1/analytics/blocked-contacts-per-day", { query: MONTH_RANGE })).json).toEqual({
        data: series(Q1_MONTHS, [0, 0, 2]),
      })
      const filtered = await api.get(`/v1/analytics/blocked-contacts-per-day${qs({ ...UTC_RANGE, inboxIds: [i2] })}`)
      expect(filtered.json).toEqual({ data: series(MARCH_1_TO_5, [0, 0, 0, 0, 0]) })
    })
  })

  describe("analytics.blockedContactsCount", () => {
    it("counts distinct blocked contacts in the range", async () => {
      const response = await api.get("/v1/analytics/blocked-contacts-count", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({ data: { count: 2 } })
      expect((await api.get("/v1/analytics/blocked-contacts-count", { query: { ...UTC_RANGE, channels: "telegram" } })).json).toEqual({
        data: { count: 0 },
      })
    })
  })

  describe("analytics.newContactsCount", () => {
    it("counts distinct contacts created in the range, with filters", async () => {
      const response = await api.get("/v1/analytics/new-contacts-count", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({ data: { count: 3 } })
      expect((await api.get("/v1/analytics/new-contacts-count", { query: { ...UTC_RANGE, channels: "telegram" } })).json).toEqual({
        data: { count: 1 },
      })
      expect((await api.get(`/v1/analytics/new-contacts-count${qs({ ...UTC_RANGE, inboxIds: [i1] })}`)).json).toEqual({
        data: { count: 2 },
      })
      // Repeated query keys build the array.
      const both = await api.get(`/v1/analytics/new-contacts-count${qs({ ...UTC_RANGE, channels: ["telegram", "messenger"] })}`)
      expect(both.json).toEqual({ data: { count: 3 } })
    })
  })

  describe("analytics.contactsCount", () => {
    it("returns the current per-inbox total; the range does not narrow it", async () => {
      const response = await api.get("/v1/analytics/contacts-count", { query: { from: "2030-01-01T00:00:00Z", to: "2030-01-02T00:00:00Z" } })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({ data: { count: 3 } })
      expect((await api.get("/v1/analytics/contacts-count", { query: { ...UTC_RANGE, channels: "telegram" } })).json).toEqual({
        data: { count: 1 },
      })
      expect((await api.get(`/v1/analytics/contacts-count${qs({ ...UTC_RANGE, inboxIds: [i1] })}`)).json).toEqual({
        data: { count: 2 },
      })
    })
  })

  describe("analytics.contactsByDimension", () => {
    it.each([
      ["channel", [{ dimension: "messenger", count: 3, uniqueContacts: 2 }, { dimension: "telegram", count: 1, uniqueContacts: 1 }]],
      ["country", [{ dimension: "VN", count: 3, uniqueContacts: 2 }, { dimension: "US", count: 1, uniqueContacts: 1 }]],
      ["source", [{ dimension: "direct", count: 3, uniqueContacts: 2 }, { dimension: "ads", count: 1, uniqueContacts: 1 }]],
      ["language", [{ dimension: "vi", count: 3, uniqueContacts: 2 }, { dimension: "en", count: 1, uniqueContacts: 1 }]],
    ])("metric new (default) by %s", async (dimension, expected) => {
      const response = await api.get("/v1/analytics/contacts-by-dimension", { query: { ...UTC_RANGE, dimension } })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({ data: expected })
    })

    it("metric total counts the balance as of `to` and drops empty groups", async () => {
      const byChannel = await api.get("/v1/analytics/contacts-by-dimension", { query: { ...UTC_RANGE, dimension: "channel", metric: "total" } })
      expect(byChannel.json).toEqual({ data: [{ dimension: "messenger", count: 4, uniqueContacts: 4 }] })
      const bySource = await api.get("/v1/analytics/contacts-by-dimension", { query: { ...UTC_RANGE, dimension: "source", metric: "total" } })
      expect(bySource.json).toEqual({
        data: [
          { dimension: "direct", count: 3, uniqueContacts: 3 },
          { dimension: "ads", count: 1, uniqueContacts: 1 },
        ],
      })
    })

    it("metric total under an inbox filter uses the per-inbox contact counts", async () => {
      const response = await api.get(
        `/v1/analytics/contacts-by-dimension${qs({ ...UTC_RANGE, dimension: "channel", metric: "total", inboxIds: [i1] })}`,
      )
      expect(response.json).toEqual({ data: [{ dimension: "messenger", count: 2, uniqueContacts: 2 }] })
    })
  })

  describe("analytics.activeContactsCount", () => {
    it("counts distinct contacts with activity hours in the range", async () => {
      const fx = await ext()
      const ws = tenant.workspaceId
      const [a, b, c] = [syntheticId(), syntheticId(), syntheticId()] as [string, string, string]
      const [a1, a2, b1, c1] = [syntheticId(), syntheticId(), syntheticId(), syntheticId()] as [string, string, string, string]
      // Own workspace slice: these contacts exist only in ContactActiveHourly.
      await fx.insertContactActiveHourly([
        { workspaceId: ws, contactId: a, contactInboxId: a1, inboxId: i1, hourBucket: "2025-03-02T10:00:00Z" },
        { workspaceId: ws, contactId: a, contactInboxId: a1, inboxId: i1, hourBucket: "2025-03-02T11:00:00Z" },
        { workspaceId: ws, contactId: b, contactInboxId: b1, inboxId: i2, hourBucket: "2025-03-03T00:00:00Z" },
        { workspaceId: ws, contactId: a, contactInboxId: a2, inboxId: i2, hourBucket: "2025-03-04T00:00:00Z" },
        { workspaceId: ws, contactId: c, contactInboxId: c1, inboxId: i1, hourBucket: "2025-02-15T00:00:00Z" },
      ])
      const response = await api.get("/v1/analytics/active-contacts-count", { query: UTC_RANGE })
      expect(response.status).toBe(200)
      expect(response.json).toEqual({ data: { count: 2 } })
      expect((await api.get(`/v1/analytics/active-contacts-count${qs({ ...UTC_RANGE, inboxIds: [i1] })}`)).json).toEqual({
        data: { count: 1 },
      })
      expect((await api.get("/v1/analytics/active-contacts-count", { query: { ...UTC_RANGE, channels: "telegram" } })).json).toEqual({
        data: { count: 2 },
      })
      // Bounds are inclusive hour buckets.
      expect(
        (await api.get("/v1/analytics/active-contacts-count", { query: { from: "2025-03-02T11:00:00Z", to: "2025-03-03T00:00:00Z" } })).json,
      ).toEqual({ data: { count: 2 } })
    })
  })

  describe("input validation", () => {
    it.each([
      ["contact-counts-per-day", { to: UTC_RANGE.to }, "from"],
      ["new-contact-counts-per-day", { from: UTC_RANGE.from }, "to"],
      ["blocked-contacts-per-day", { ...UTC_RANGE, channels: "fax" }, "channels.0"],
      ["blocked-contacts-count", { ...UTC_RANGE, inboxIds: "abc" }, "inboxIds.0"],
      ["new-contacts-count", { ...UTC_RANGE, inboxIds: "1234567890123456789" }, "inboxIds.0"],
      ["contacts-count", { from: UTC_RANGE.from }, "to"],
      ["active-contacts-count", { to: UTC_RANGE.to }, "from"],
      ["contacts-by-dimension", { ...UTC_RANGE }, "dimension"],
      ["contacts-by-dimension", { ...UTC_RANGE, dimension: "city" }, "dimension"],
      ["contacts-by-dimension", { ...UTC_RANGE, dimension: "channel", metric: "all" }, "metric"],
      ["contacts-by-dimension", { ...UTC_RANGE, dimension: "country", metric: "total" }, "metric"],
    ])("%s rejects %j (422)", async (route, query, path) => {
      expectInvalid(await api.get(`/v1/analytics/${route}`, { query }), path)
    })

    it("refuses metric total by source under an inbox filter (422)", async () => {
      expectInvalid(
        await api.get(`/v1/analytics/contacts-by-dimension${qs({ ...UTC_RANGE, dimension: "source", metric: "total", inboxIds: [i1] })}`),
        "metric",
      )
    })

    it("refuses more than 200 inbox ids (422)", async () => {
      const ids = Array.from({ length: 201 }, (_, index) => String(index + 1))
      expectInvalid(await api.get(`/v1/analytics/new-contacts-count${qs({ ...UTC_RANGE, inboxIds: ids })}`), "inboxIds")
    })
  })
})

describe("analytics time range floor", () => {
  it("clamps `from` to 2020-01-01", async () => {
    const { tenant, api } = await tenantApi("an-floor")
    const ws = tenant.workspaceId
    await (await ext()).insertContactEvents([
      { workspaceId: ws, contactId: syntheticId(), eventType: "contact_created", occurredAt: "2019-06-01T00:00:00Z" },
      { workspaceId: ws, contactId: syntheticId(), eventType: "contact_created", occurredAt: "2020-01-01T00:00:00Z" },
      { workspaceId: ws, contactId: syntheticId(), eventType: "contact_created", occurredAt: "2020-02-01T00:00:00Z" },
    ])
    const response = await api.get("/v1/analytics/new-contacts-count", { query: { from: "2019-01-01T00:00:00Z", to: "2020-12-31T00:00:00Z" } })
    expect(response.json).toEqual({ data: { count: 2 } })
  })
})

describe("analytics.macActiveContactCount", () => {
  it("returns the MAC count of the current billing period", async () => {
    const { tenant, api } = await tenantApi("an-mac")
    const now = Date.now()
    const isoDays = (days: number) => new Date(now + days * 24 * 3600_000).toISOString()
    const fx = await ext()
    await fx.insertWorkspaceMac({ workspaceId: tenant.workspaceId, ownerId: tenant.userId, periodStart: isoDays(-40), periodEnd: isoDays(-10), macCount: 99 })
    await fx.insertWorkspaceMac({ workspaceId: tenant.workspaceId, ownerId: tenant.userId, periodStart: isoDays(-10), periodEnd: isoDays(20), macCount: 7 })
    const response = await api.get("/v1/analytics/mac/active-count")
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ data: { macCount: 7 } })
  })

  it("is 0 without a current period", async () => {
    const { api } = await tenantApi("an-mac-none")
    expect((await api.get("/v1/analytics/mac/active-count")).json).toEqual({ data: { macCount: 0 } })
  })
})

describe("contact analytics fed by the public API (end to end)", () => {
  it("counts contacts created and blocked through contacts.create / contacts.block", async () => {
    const { tenant, api } = await tenantApi("an-e2e")
    const { inboxId } = await (await ext()).createInbox({ workspaceId: tenant.workspaceId, name: uniqueName("E2E"), channel: "messenger" })
    const from = new Date(Date.now() - 60_000).toISOString()
    const created: string[] = []
    for (const firstName of ["Ada", "Bo"]) {
      const response = await api.post<{ id: string }>("/v1/contacts", {
        channel: "messenger",
        inboxId,
        contactId: `psid-${uniqueId()}`,
        email: "",
        gender: "unknown",
        firstName,
      })
      // contacts.create declares no successStatus: 200.
      expect(response.status).toBe(200)
      created.push(response.json.id)
    }
    // The identifier needs its prefix (`id:<contact id>`).
    expect((await api.post(`/v1/contacts/id:${created[0]}/block`, {})).status).toBe(204)

    // InboxContactStat is kept by a trigger: no worker involved.
    expect((await api.get("/v1/analytics/contacts-count", { query: { from, to: new Date().toISOString() } })).json).toEqual({
      data: { count: 2 },
    })
    // The analytics events arrive through the event bus and the worker. A
    // new `to` on every poll avoids the 120 s response cache.
    let attempt = 0
    const window = () => ({ from, to: new Date(Date.now() + 60_000 + (attempt += 1)).toISOString() })
    await eventually(
      async () => {
        expect((await api.get("/v1/analytics/new-contacts-count", { query: window() })).json).toEqual({ data: { count: 2 } })
        expect((await api.get("/v1/analytics/blocked-contacts-count", { query: window() })).json).toEqual({ data: { count: 1 } })
      },
      { timeoutMs: 25_000, intervalMs: 500, description: "contact events recorded" },
    )
    const byChannel = await api.get("/v1/analytics/contacts-by-dimension", { query: { ...window(), dimension: "channel" } })
    expect(byChannel.json).toEqual({ data: [{ dimension: "messenger", count: 2, uniqueContacts: 2 }] })
    const bySource = await api.get("/v1/analytics/contacts-by-dimension", { query: { ...window(), dimension: "source" } })
    expect(bySource.json).toEqual({ data: [{ dimension: "direct", count: 2, uniqueContacts: 2 }] })
    const byInbox = await api.get(`/v1/analytics/new-contacts-count${qs({ ...window(), inboxIds: [inboxId] })}`)
    expect(byInbox.json).toEqual({ data: { count: 2 } })
  })
})
