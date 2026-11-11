// googleAds.getConnection / getStats / listEvents: read-only views of the
// workspace's Google Ads connection, consent and conversion history. The
// connection, consent and event rows have no public write path: they come from
// the P1.9 fixture extension (terminal event statuses only, so no background
// job sends anything to Google).

import { describe, expect, it } from "vitest"
import type { P109GoogleAdsEventRow } from "../../src/fixtures/ext/p1-09.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import { uniqueId } from "../../src/unique.ts"
import { expectInvalid, ext, tenantApi, uniqueDigits } from "./helpers-ig.ts"

const NOT_CONNECTED = {
  connected: false,
  readiness: null,
  customerId: null,
  descriptiveName: null,
  currencyCode: null,
  uploadMethod: null,
  acceptedCustomerDataTerms: null,
  setupError: null,
  conversionActions: [],
  conversionActionsSyncedAt: null,
}

const ABSENT_CONSENT = {
  status: "absent",
  adUserData: { type: "notProvided", template: null },
  adPersonalization: { type: "notProvided", template: null },
}

const ACTIONS = [
  { id: "111", name: "Purchase", category: "PURCHASE", status: "ENABLED", countingType: "ONE_PER_CLICK", attributionModel: "GOOGLE_SEARCH_ATTRIBUTION_DATA_DRIVEN" },
  // Cached before the attribution model was read: no key at all.
  { id: "222", name: "Lead", category: "SUBMIT_LEAD_FORM", status: "ENABLED", countingType: "MANY_PER_CLICK" },
]

describe("googleAds.getConnection", () => {
  it("not connected: connected=false, nulls, no actions, absent consent", async () => {
    const { api } = await tenantApi("p109-gads-none")
    const response = await api.get("/v1/google-ads/connection")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ ...NOT_CONNECTED, consent: ABSENT_CONSENT })
  })

  it("ready connection: account, Data Manager upload method, synced actions without credentials", async () => {
    const { tenant, api } = await tenantApi("p109-gads-ready")
    const customerId = uniqueDigits().slice(0, 10)
    const syncedAt = "2026-01-10T08:00:00.000Z"
    await (await ext()).createGoogleAdsConnection({
      workspaceId: tenant.workspaceId,
      customerId,
      descriptiveName: "Parity Ads",
      currencyCode: "EUR",
      connectionStatus: "connected",
      conversionCustomerId: customerId,
      acceptedCustomerDataTerms: true,
      conversionActions: ACTIONS,
      conversionActionsSyncedAt: syncedAt,
      setupError: null,
    })
    const response = await api.get("/v1/google-ads/connection")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      connected: true,
      readiness: "ready",
      customerId,
      descriptiveName: "Parity Ads",
      currencyCode: "EUR",
      // No `metadata.uploadMethod` in the auth: Data Manager.
      uploadMethod: "dataManager",
      acceptedCustomerDataTerms: true,
      setupError: null,
      conversionActions: [
        {
          id: "111",
          name: "Purchase",
          category: "PURCHASE",
          status: "ENABLED",
          countingType: "ONE_PER_CLICK",
          attributionModel: "GOOGLE_SEARCH_ATTRIBUTION_DATA_DRIVEN",
        },
        { id: "222", name: "Lead", category: "SUBMIT_LEAD_FORM", status: "ENABLED", countingType: "MANY_PER_CLICK", attributionModel: null },
      ],
      conversionActionsSyncedAt: syncedAt,
      consent: ABSENT_CONSENT,
    })
    expect(response.text).not.toContain("parity-google-ads")
  })

  it("setup_incomplete, needs_reauth and legacy upload method", async () => {
    const incomplete = await tenantApi("p109-gads-incomplete")
    await (await ext()).createGoogleAdsConnection({
      workspaceId: incomplete.tenant.workspaceId,
      customerId: uniqueDigits().slice(0, 10),
      descriptiveName: null,
      currencyCode: null,
      connectionStatus: "degraded",
      conversionCustomerId: null,
      acceptedCustomerDataTerms: null,
      conversionActions: null,
      conversionActionsSyncedAt: null,
      setupError: "sync_failed",
    })
    expect((await incomplete.api.get("/v1/google-ads/connection")).json).toMatchObject({
      connected: true,
      readiness: "setup_incomplete",
      descriptiveName: null,
      currencyCode: null,
      acceptedCustomerDataTerms: null,
      setupError: "sync_failed",
      conversionActions: [],
      conversionActionsSyncedAt: null,
    })

    const reauth = await tenantApi("p109-gads-reauth")
    const customerId = uniqueDigits().slice(0, 10)
    await (await ext()).createGoogleAdsConnection({
      workspaceId: reauth.tenant.workspaceId,
      customerId,
      descriptiveName: "Old",
      currencyCode: "USD",
      uploadMethod: "legacy",
      connectionStatus: "needs_reauth",
      conversionCustomerId: customerId,
      acceptedCustomerDataTerms: false,
      conversionActions: [],
      conversionActionsSyncedAt: "2026-01-01T00:00:00.000Z",
      setupError: null,
    })
    expect((await reauth.api.get("/v1/google-ads/connection")).json).toMatchObject({
      connected: true,
      readiness: "needs_reauth",
      uploadMethod: "legacy",
      acceptedCustomerDataTerms: false,
      conversionActions: [],
    })
  })

  it("consent is read on its own: stored values, variable template, unreadable document", async () => {
    const { tenant, api } = await tenantApi("p109-gads-consent")
    await (await ext()).setGoogleAdsSettings({
      workspaceId: tenant.workspaceId,
      settings: {
        version: 1,
        consent: {
          adUserData: { type: "granted" },
          adPersonalization: { type: "variable", template: "{{custom.ads_consent}}" },
        },
      },
    })
    expect((await api.get("/v1/google-ads/connection")).json).toEqual({
      ...NOT_CONNECTED,
      consent: {
        status: "ok",
        adUserData: { type: "granted", template: null },
        adPersonalization: { type: "variable", template: "{{custom.ads_consent}}" },
      },
    })

    const broken = await tenantApi("p109-gads-consent-bad")
    await (await ext()).setGoogleAdsSettings({
      workspaceId: broken.tenant.workspaceId,
      settings: { version: 2, consent: {} },
    })
    expect((await broken.api.get("/v1/google-ads/connection")).json).toEqual({
      ...NOT_CONNECTED,
      consent: { status: "invalid", adUserData: null, adPersonalization: null },
    })
  })
})

// ---------------------------------------------------------------------------
// Event rows
// ---------------------------------------------------------------------------

const clickId = () => `Cj0KCQiA${uniqueId()}`

const v1Options = (overrides: Record<string, unknown> = {}) => ({
  version: 1,
  identity: { version: 1, configuredPolicy: "click", effectivePolicy: "click", keySource: "click", id: null },
  timeSource: "recorded",
  consent: {
    adUserData: { status: "granted", source: "fixed" },
    adPersonalization: { status: null, source: "notProvided" },
  },
  ...overrides,
})

const event = (overrides: Partial<P109GoogleAdsEventRow>): P109GoogleAdsEventRow => ({
  conversionActionId: "111",
  conversionActionName: "Purchase",
  conversionActionCategory: "PURCHASE",
  channel: "whatsapp",
  status: "processed",
  clickIdType: "gclid",
  clickId: clickId(),
  occurredAt: "2026-01-01T10:00:00.000Z",
  transactionId: `tx-${uniqueId()}`,
  uploadMethod: "dataManager",
  options: v1Options(),
  ...(overrides.status === undefined || overrides.status === "processed"
    ? { requestId: `req-${uniqueId()}`, sentAt: "2026-01-01T10:05:00.000Z", processingStatus: "success" as const }
    : {}),
  ...overrides,
})

const insertEvents = async (tenant: Tenant, rows: P109GoogleAdsEventRow[]) =>
  (await ext()).insertGoogleAdsEvents({
    workspaceId: tenant.workspaceId,
    integrationGoogleAdsId: null,
    customerId: "1234567890",
    rows,
  })

const counts = (values: Partial<Record<string, number>> = {}) => ({
  pending: 0,
  sending: 0,
  sent: 0,
  processed: 0,
  failed: 0,
  skipped_no_account: 0,
  skipped_expired: 0,
  ...values,
})

/** The stats fixture: 7 events in 2026-01-01..03 (UTC) and one on 2025-12-31. */
const seedStats = async (tenant: Tenant) =>
  insertEvents(tenant, [
    event({ occurredAt: "2026-01-01T10:00:00.000Z", channel: "whatsapp", conversionActionName: "Purchase old", value: "10.50", currency: "EUR" }),
    event({ occurredAt: "2026-01-01T12:00:00.000Z", channel: "messenger", value: "2.00", currency: "EUR" }),
    event({
      occurredAt: "2026-01-02T09:00:00.000Z",
      conversionActionId: "222",
      conversionActionName: "Lead",
      conversionActionCategory: "SUBMIT_LEAD_FORM",
      status: "failed",
      failureStage: "delivery",
      error: "rejected",
    }),
    event({
      occurredAt: "2026-01-02T10:00:00.000Z",
      conversionActionId: "222",
      conversionActionName: "Lead",
      conversionActionCategory: "SUBMIT_LEAD_FORM",
      status: "failed",
      failureStage: "processing",
      processingStatus: "failed",
      requestId: `req-${uniqueId()}`,
      sentAt: "2026-01-02T10:01:00.000Z",
    }),
    event({
      occurredAt: "2026-01-02T11:00:00.000Z",
      conversionActionId: "333",
      conversionActionName: null,
      conversionActionCategory: "DEFAULT",
      channel: "zalo",
      status: "failed",
      failureStage: null,
    }),
    event({
      occurredAt: "2026-01-03T05:00:00.000Z",
      conversionActionId: "222",
      conversionActionName: "Lead form",
      conversionActionCategory: "SUBMIT_LEAD_FORM",
      channel: "messenger",
      value: "5.25",
      currency: "USD",
    }),
    event({ occurredAt: "2026-01-03T23:30:00.000Z", channel: "zalo", status: "skipped_no_account" }),
    event({ occurredAt: "2025-12-31T23:00:00.000Z", value: "100.00", currency: "EUR" }),
  ])

describe("googleAds.getStats", () => {
  it("empty workspace: zero-filled days and null delivery rate", async () => {
    const { api } = await tenantApi("p109-gads-stats-empty")
    const response = await api.get("/v1/google-ads/stats", { query: { from: "2026-01-01", to: "2026-01-03" } })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      range: { from: "2026-01-01", to: "2026-01-03", tz: "UTC" },
      totals: { ...counts(), total: 0, deliveryRate: null },
      failuresByStage: { delivery: 0, processing: 0, timeout: 0, unknown: 0 },
      confirmedValue: [],
      timeseries: ["2026-01-01", "2026-01-02", "2026-01-03"].map((date) => ({ date, counts: counts() })),
      byAction: [],
      byActionTruncated: false,
      byChannel: [],
    })
  })

  it("counts per status, day, action and channel; confirmed value per currency", async () => {
    const { tenant, api } = await tenantApi("p109-gads-stats")
    await seedStats(tenant)
    const response = await api.get("/v1/google-ads/stats", { query: { from: "2026-01-01", to: "2026-01-03" } })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      range: { from: "2026-01-01", to: "2026-01-03", tz: "UTC" },
      totals: { ...counts({ processed: 3, failed: 3, skipped_no_account: 1 }), total: 7, deliveryRate: 0.5 },
      failuresByStage: { delivery: 1, processing: 1, timeout: 0, unknown: 1 },
      confirmedValue: [
        { currency: "EUR", value: "12.50", count: 2 },
        { currency: "USD", value: "5.25", count: 1 },
      ],
      timeseries: [
        { date: "2026-01-01", counts: counts({ processed: 2 }) },
        { date: "2026-01-02", counts: counts({ failed: 3 }) },
        { date: "2026-01-03", counts: counts({ processed: 1, skipped_no_account: 1 }) },
      ],
      // Busiest first, ties by action id; name and category of the newest event.
      byAction: [
        { conversionActionId: "111", name: "Purchase", category: "PURCHASE", total: 3, counts: counts({ processed: 2, skipped_no_account: 1 }) },
        { conversionActionId: "222", name: "Lead form", category: "SUBMIT_LEAD_FORM", total: 3, counts: counts({ processed: 1, failed: 2 }) },
        { conversionActionId: "333", name: null, category: "DEFAULT", total: 1, counts: counts({ failed: 1 }) },
      ],
      byActionTruncated: false,
      // Busiest first, ties by channel name.
      byChannel: [
        { channel: "whatsapp", total: 3, counts: counts({ processed: 1, failed: 2 }) },
        { channel: "messenger", total: 2, counts: counts({ processed: 2 }) },
        { channel: "zalo", total: 2, counts: counts({ failed: 1, skipped_no_account: 1 }) },
      ],
    })
  })

  it("filters by channel and conversion action; buckets days in `tz`", async () => {
    const { tenant, api } = await tenantApi("p109-gads-stats-filter")
    await seedStats(tenant)
    const whatsapp = await api.get<Record<string, unknown>>("/v1/google-ads/stats", {
      query: { from: "2026-01-01", to: "2026-01-03", channel: "whatsapp" },
    })
    expect(whatsapp.json).toMatchObject({
      totals: { ...counts({ processed: 1, failed: 2 }), total: 3, deliveryRate: 1 / 3 },
      confirmedValue: [{ currency: "EUR", value: "10.50", count: 1 }],
      byChannel: [{ channel: "whatsapp", total: 3 }],
    })
    const action = await api.get<Record<string, unknown>>("/v1/google-ads/stats", {
      query: { from: "2026-01-01", to: "2026-01-03", conversionActionId: "222" },
    })
    expect(action.json).toMatchObject({
      totals: { ...counts({ processed: 1, failed: 2 }), total: 3 },
      byAction: [{ conversionActionId: "222", total: 3 }],
      confirmedValue: [{ currency: "USD", value: "5.25", count: 1 }],
    })
    // 2026-01-02 in Tokyo is 2026-01-01T15:00Z..2026-01-02T15:00Z.
    const tokyo = await api.get<Record<string, unknown>>("/v1/google-ads/stats", {
      query: { from: "2026-01-02", to: "2026-01-02", tz: "Asia/Tokyo" },
    })
    expect(tokyo.status, tokyo.text).toBe(200)
    expect(tokyo.json).toMatchObject({
      range: { from: "2026-01-02", to: "2026-01-02", tz: "Asia/Tokyo" },
      totals: { ...counts({ failed: 3 }), total: 3 },
      timeseries: [{ date: "2026-01-02", counts: counts({ failed: 3 }) }],
    })
  })

  it("normalises ranges: inverted -> last 7 days, oversized -> 366 days ending at `to`, unknown tz -> UTC", async () => {
    const { api } = await tenantApi("p109-gads-stats-range")
    const day = (offset: number, base = new Date()) => {
      const date = new Date(`${base.toISOString().slice(0, 10)}T00:00:00.000Z`)
      date.setUTCDate(date.getUTCDate() + offset)
      return date.toISOString().slice(0, 10)
    }
    const before = day(0)
    const inverted = await api.get<{ range: { from: string; to: string }; timeseries: unknown[] }>(
      "/v1/google-ads/stats",
      { query: { from: "2026-02-10", to: "2026-02-01" } },
    )
    expect(inverted.status).toBe(200)
    const today = inverted.json.range.to
    expect([before, day(0)]).toContain(today)
    expect(inverted.json.range.from).toBe(day(-6, new Date(`${today}T00:00:00.000Z`)))
    expect(inverted.json.timeseries).toHaveLength(7)

    const oversized = await api.get<{ range: { from: string; to: string }; timeseries: unknown[] }>(
      "/v1/google-ads/stats",
      { query: { from: "2024-01-01", to: "2026-01-10" } },
    )
    expect(oversized.json.range).toEqual({ from: "2025-01-10", to: "2026-01-10", tz: "UTC" })
    expect(oversized.json.timeseries).toHaveLength(366)

    const unknownTz = await api.get<{ range: Record<string, string> }>("/v1/google-ads/stats", {
      query: { from: "2026-01-01", to: "2026-01-01", tz: "Mars/Olympus" },
    })
    expect(unknownTz.status).toBe(200)
    expect(unknownTz.json.range).toEqual({ from: "2026-01-01", to: "2026-01-01", tz: "UTC" })
  })

  it("validates the query (422 invalidRequestData)", async () => {
    const { api } = await tenantApi("p109-gads-stats-invalid")
    const ok = { from: "2026-01-01", to: "2026-01-02" }
    expectInvalid(await api.get("/v1/google-ads/stats", { query: { to: "2026-01-02" } }), "from")
    expectInvalid(await api.get("/v1/google-ads/stats", { query: { ...ok, from: "2026/01/01" } }), "from")
    expectInvalid(await api.get("/v1/google-ads/stats", { query: { ...ok, to: "yesterday" } }), "to")
    expectInvalid(await api.get("/v1/google-ads/stats", { query: { ...ok, channel: "telegram" } }), "channel")
    expectInvalid(await api.get("/v1/google-ads/stats", { query: { ...ok, conversionActionId: "abc" } }), "conversionActionId")
  })
})

describe("googleAds.listEvents", () => {
  it("newest first, masked click ids, redacted errors and option snapshots", async () => {
    const { tenant, api } = await tenantApi("p109-gads-events")
    const longClick = clickId()
    const txId = `tx-${uniqueId()}`
    const reqId = `req-${uniqueId()}`
    const {
      ids: [first, second, third],
    } = await insertEvents(tenant, [
      event({
        occurredAt: "2026-02-01T10:00:00.000Z",
        clickId: longClick,
        value: "9.99",
        currency: "USD",
        sentAt: "2026-02-01T10:05:00.000Z",
      }),
      event({
        occurredAt: "2026-02-02T10:00:00.000Z",
        channel: "messenger",
        conversionActionName: null,
        status: "failed",
        failureStage: "delivery",
        uploadMethod: "legacy",
        clickId: longClick,
        transactionId: txId,
        requestId: reqId,
        error: `Click ${longClick} for ${txId} rejected (request ${reqId})`,
        options: {
          ...v1Options({
            identity: { version: 1, configuredPolicy: "id", effectivePolicy: "id", keySource: "explicit", id: "order-42" },
            timeSource: "provided",
            consent: {
              adUserData: { status: "denied", source: "variable" },
              adPersonalization: { status: "granted", source: "fixed" },
            },
          }),
          version: 2,
          matching: { status: "enabled", email: "{{email}}", phone: null },
          customerProperties: { status: "withheldConsent", customerType: "NEW", customerValueBucket: null },
        },
      }),
      event({
        occurredAt: "2026-02-03T10:00:00.000Z",
        channel: "zalo",
        status: "skipped_expired",
        clickIdType: "gbraid",
        clickId: "abcdefgh12",
        options: null,
      }),
    ])
    const response = await api.get<{ data: Array<Record<string, unknown>>; pageCount: number }>("/v1/google-ads/events")
    expect(response.status, response.text).toBe(200)
    expect(response.json.pageCount).toBe(1)
    const masked = `${longClick.slice(0, 4)}…${longClick.slice(-4)}`
    expect(response.json.data).toEqual([
      {
        id: third,
        status: "skipped_expired",
        failureStage: null,
        processingStatus: null,
        error: null,
        channel: "zalo",
        conversionActionId: "111",
        conversionActionName: "Purchase",
        uploadMethod: "dataManager",
        clickIdType: "gbraid",
        maskedClickId: "abcd…gh12",
        occurredAt: "2026-02-03T10:00:00.000Z",
        sentAt: null,
        value: null,
        currency: null,
        identity: null,
        conversionTimeProvided: false,
        customerMatching: null,
        customerProperties: null,
        consentSnapshot: null,
      },
      {
        id: second,
        status: "failed",
        failureStage: "delivery",
        processingStatus: null,
        error: "Click [redacted] for [redacted] rejected (request [redacted])",
        channel: "messenger",
        conversionActionId: "111",
        conversionActionName: null,
        uploadMethod: "legacy",
        clickIdType: "gclid",
        maskedClickId: masked,
        occurredAt: "2026-02-02T10:00:00.000Z",
        sentAt: null,
        value: null,
        currency: null,
        identity: { mode: "id", id: "order-42" },
        conversionTimeProvided: true,
        customerMatching: { status: "enabled", fields: ["email"] },
        customerProperties: { status: "withheldConsent", customerType: "NEW", customerValueBucket: null },
        // A delivery-stage failure may or may not have reached Google; legacy has no ad personalization.
        consentSnapshot: { delivery: "unknown", adUserData: "denied", adPersonalization: "notSupported" },
      },
      {
        id: first,
        status: "processed",
        failureStage: null,
        processingStatus: "success",
        error: null,
        channel: "whatsapp",
        conversionActionId: "111",
        conversionActionName: "Purchase",
        uploadMethod: "dataManager",
        clickIdType: "gclid",
        maskedClickId: masked,
        occurredAt: "2026-02-01T10:00:00.000Z",
        sentAt: "2026-02-01T10:05:00.000Z",
        value: "9.99",
        currency: "USD",
        identity: { mode: "click", id: null },
        conversionTimeProvided: false,
        customerMatching: null,
        customerProperties: null,
        consentSnapshot: { delivery: "sent", adUserData: "granted", adPersonalization: "notProvided" },
      },
    ])
    expect(response.text).not.toContain(longClick)
  })

  it("filters by status, channel, action and time window; pages (empty list has pageCount 1)", async () => {
    const { tenant, api } = await tenantApi("p109-gads-events-filter")
    const empty = await api.get("/v1/google-ads/events")
    expect(empty.json).toEqual({ data: [], pageCount: 1 })
    const { ids } = await insertEvents(tenant, [
      event({ occurredAt: "2026-03-01T00:00:00.000Z" }),
      event({ occurredAt: "2026-03-02T00:00:00.000Z", channel: "messenger", status: "failed", failureStage: "timeout" }),
      event({ occurredAt: "2026-03-03T00:00:00.000Z", conversionActionId: "222", status: "skipped_no_account" }),
    ])
    const [a, b, c] = ids as [string, string, string]
    const listIds = async (query: Record<string, string | number>) => {
      const response = await api.get<{ data: Array<{ id: string }>; pageCount: number }>("/v1/google-ads/events", {
        query,
      })
      expect(response.status, response.text).toBe(200)
      return { ids: response.json.data.map((item) => item.id), pageCount: response.json.pageCount }
    }
    expect(await listIds({})).toEqual({ ids: [c, b, a], pageCount: 1 })
    expect((await listIds({ status: "failed" })).ids).toEqual([b])
    expect((await listIds({ channel: "messenger" })).ids).toEqual([b])
    expect((await listIds({ conversionActionId: "222" })).ids).toEqual([c])
    expect((await listIds({ since: "2026-03-02T00:00:00.000Z" })).ids).toEqual([c, b])
    expect((await listIds({ until: "2026-03-02T00:00:00.000Z" })).ids).toEqual([b, a])
    expect(await listIds({ perPage: 2 })).toEqual({ ids: [c, b], pageCount: 2 })
    expect(await listIds({ perPage: 2, page: 2 })).toEqual({ ids: [a], pageCount: 2 })

    expectInvalid(await api.get("/v1/google-ads/events", { query: { status: "lost" } }), "status")
    expectInvalid(await api.get("/v1/google-ads/events", { query: { channel: "tiktok" } }), "channel")
    expectInvalid(await api.get("/v1/google-ads/events", { query: { conversionActionId: "x1" } }), "conversionActionId")
    expectInvalid(await api.get("/v1/google-ads/events", { query: { since: "not a date" } }), "since")
    expectInvalid(await api.get("/v1/google-ads/events", { query: { perPage: 51 } }), "perPage")
  })
})
