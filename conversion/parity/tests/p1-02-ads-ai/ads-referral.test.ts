// Ad touches (ContactReferralEvent): ads.referralStats, ads.listReferralAds.

import { describe, expect, it } from "vitest"
import { createTenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { uniqueId } from "../../src/unique.ts"
import { adsFixtures, expectValidationIssue, uniqueDigits } from "./ads-helpers.ts"

/**
 * Contact A: touches ad X (Oct 2, "Summer sale") then ad Y (Oct 3).
 * Contact B: touches ad Y (Oct 4, "Winter"). Contact C: ad X in September.
 * A lead is linked to A's touch of X; a 15.5 USD purchase to B's touch of Y.
 */
const seed = async (label: string) => {
  const tenant = await createTenant({ label })
  const ext = await adsFixtures()
  const pageId = uniqueDigits()
  const page = await ext.createMessengerChannel({ workspaceId: tenant.workspaceId, pageId, name: `Page ${pageId}` })
  const adX = uniqueDigits(12)
  const adY = uniqueDigits(12)
  const contact = (name: string) =>
    ext.createContactInbox({
      workspaceId: tenant.workspaceId,
      inboxId: page.inboxId,
      channel: "messenger",
      sourceId: `psid-${name}-${uniqueId()}`,
      firstName: name,
    })
  const a = await contact("A")
  const b = await contact("B")
  const c = await contact("C")
  const touch = (who: { contactId: string; contactInboxId: string }, adId: string, occurredAt: string, adTitle?: string | null) =>
    ext.createReferralEvent({
      workspaceId: tenant.workspaceId,
      contactId: who.contactId,
      contactInboxId: who.contactInboxId,
      channel: "messenger",
      adId,
      occurredAt,
      adTitle,
      dedupKey: `touch-${uniqueId()}`,
    })
  const aX = await touch(a, adX, "2026-10-02T10:00:00.000Z", "Summer sale")
  await touch(a, adY, "2026-10-03T10:00:00.000Z", null)
  const bY = await touch(b, adY, "2026-10-04T10:00:00.000Z", "Winter")
  await touch(c, adX, "2026-09-15T10:00:00.000Z", "Old title")
  const base = { workspaceId: tenant.workspaceId, channel: "messenger" as const, integrationId: page.integrationId }
  await ext.createConversionEvent({
    ...base,
    eventType: "lead",
    occurredAt: "2026-10-02T11:00:00.000Z",
    sourceEventId: `lead-${uniqueId()}`,
    adId: adX,
    contactInboxId: a.contactInboxId,
    referralEventId: aX.referralEventId,
  })
  await ext.createConversionEvent({
    ...base,
    eventType: "purchase",
    occurredAt: "2026-10-04T11:00:00.000Z",
    sourceEventId: `purchase-${uniqueId()}`,
    adId: adY,
    contactInboxId: b.contactInboxId,
    referralEventId: bY.referralEventId,
    currency: "USD",
    value: "15.5",
  })
  return { tenant, api: apiClient(tenant.token), adX, adY }
}

describe("ads.referralStats (GET /v1/ads/referral-stats)", () => {
  it("reports reach, first touches and linked conversions per ad", async () => {
    const { api, adX, adY } = await seed("referral-stats")
    const query = { from: "2026-10-01", to: "2026-10-07" }
    const response = await api.get("/v1/ads/referral-stats", { query })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      data: [
        {
          adId: adY,
          title: "Winter",
          anyTouchContacts: 2,
          firstTouchContacts: 1,
          leads: 0,
          purchases: 1,
          revenue: [{ currency: "USD", amount: "15.5" }],
        },
        {
          adId: adX,
          title: "Summer sale",
          anyTouchContacts: 1,
          firstTouchContacts: 1,
          leads: 1,
          purchases: 0,
          revenue: [],
        },
      ],
    })

    const limited = await api.get<{ data: Array<{ adId: string }> }>("/v1/ads/referral-stats", { query: { ...query, limit: 1 } })
    expect(limited.json.data.map((row) => row.adId)).toEqual([adY])
    const september = await api.get<{ data: Array<Record<string, unknown>> }>("/v1/ads/referral-stats", {
      query: { from: "2026-09-01", to: "2026-09-30" },
    })
    expect(september.json.data).toEqual([
      { adId: adX, title: "Old title", anyTouchContacts: 1, firstTouchContacts: 1, leads: 0, purchases: 0, revenue: [] },
    ])
    const instagram = await api.get("/v1/ads/referral-stats", { query: { ...query, channel: "instagram" } })
    expect(instagram.json).toEqual({ data: [] })
    // Default range: the last 7 days (nothing seeded there).
    const defaults = await api.get("/v1/ads/referral-stats")
    expect(defaults.json).toEqual({ data: [] })
  })

  it("rejects bad input with 422", async () => {
    const tenant = await createTenant({ label: "referral-stats-422" })
    const api = apiClient(tenant.token)
    expectValidationIssue(await api.get("/v1/ads/referral-stats", { query: { from: "20261001" } }), ["from"])
    expectValidationIssue(await api.get("/v1/ads/referral-stats", { query: { limit: 101 } }), ["limit"])
    expectValidationIssue(await api.get("/v1/ads/referral-stats", { query: { limit: 0 } }), ["limit"])
    expectValidationIssue(await api.get("/v1/ads/referral-stats", { query: { channel: "facebook" } }), ["channel"])
    expectValidationIssue(await api.get("/v1/ads/referral-stats", { query: { tz: "x".repeat(65) } }), ["tz"])
  })
})

describe("ads.listReferralAds (GET /v1/ads/referral-ads)", () => {
  it("lists recent ads newest first with their latest headline", async () => {
    const { tenant, api, adX, adY } = await seed("referral-ads")
    const response = await api.get("/v1/ads/referral-ads")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      data: [
        { id: adY, title: "Winter" },
        { id: adX, title: "Summer sale" },
      ],
    })
    const searched = await api.get("/v1/ads/referral-ads", { query: { search: "SUMMER" } })
    expect(searched.json).toEqual({ data: [{ id: adX, title: "Summer sale" }] })
    const byId = await api.get("/v1/ads/referral-ads", { query: { search: adY } })
    expect(byId.json).toEqual({ data: [{ id: adY, title: "Winter" }] })
    const limited = await api.get("/v1/ads/referral-ads", { query: { limit: 1 } })
    expect(limited.json).toEqual({ data: [{ id: adY, title: "Winter" }] })

    // Ad ids that are not positive bigints are not listed.
    const ext = await adsFixtures()
    const pageId = uniqueDigits()
    const page = await ext.createMessengerChannel({ workspaceId: tenant.workspaceId, pageId, name: "Other" })
    const other = await ext.createContactInbox({
      workspaceId: tenant.workspaceId,
      inboxId: page.inboxId,
      channel: "messenger",
      sourceId: `psid-${uniqueId()}`,
    })
    await ext.createReferralEvent({
      workspaceId: tenant.workspaceId,
      contactId: other.contactId,
      contactInboxId: other.contactInboxId,
      channel: "messenger",
      adId: "not-a-number",
      occurredAt: "2026-10-06T10:00:00.000Z",
      adTitle: "Bad id",
      dedupKey: `touch-${uniqueId()}`,
    })
    const after = await api.get("/v1/ads/referral-ads")
    expect(after.json).toEqual(response.json)

    const empty = await apiClient((await createTenant({ label: "referral-ads-empty" })).token).get("/v1/ads/referral-ads")
    expect(empty.json).toEqual({ data: [] })
  })

  it("rejects bad input with 422", async () => {
    const tenant = await createTenant({ label: "referral-ads-422" })
    const api = apiClient(tenant.token)
    expectValidationIssue(await api.get("/v1/ads/referral-ads", { query: { limit: 0 } }), ["limit"])
    expectValidationIssue(await api.get("/v1/ads/referral-ads", { query: { limit: 101 } }), ["limit"])
    expectValidationIssue(await api.get("/v1/ads/referral-ads", { query: { search: "x".repeat(101) } }), ["search"])
  })
})
