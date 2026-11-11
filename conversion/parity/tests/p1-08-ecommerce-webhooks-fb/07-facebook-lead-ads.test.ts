// facebookLeadAds.* — automations that turn a Page's lead-form submissions
// into contacts. Pages are fixture Messenger channels; every Graph call
// (debug_token, leadgen_forms, me/subscribed_apps) goes to the fake upstream.
// Scripts on the shared paths (`/debug_token`, `/me/subscribed_apps`) are
// limited with `times` and consumed by the test that adds them.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import {
  type Api,
  expectError,
  expectInvalid,
  ext,
  GRAPH,
  graphPath,
  keysOf,
  messengerPage,
  qs,
  reEscape,
  sortQuery,
  tenantApi,
  uniqueDigits,
  uniqueName,
  upstreamCalls,
} from "./helpers.ts"

const AUTOMATION_KEYS = [
  "createdAt",
  "fieldMapping",
  "formId",
  "formName",
  "id",
  "leadsHandledCount",
  "name",
  "pageId",
  "pageName",
  "updatedAt",
]

/** `LEAD_ADS_PAGE_SUBSCRIBE_FIELDS` (integrations/messenger/src/apis/page.ts). */
const LEAD_FIELDS = [
  "messages",
  "messaging_postbacks",
  "messaging_optins",
  "message_reads",
  "messaging_referrals",
  "message_echoes",
  "messaging_customer_information",
  "messaging_feedback",
  "messaging_policy_enforcement",
  "feed",
  "inbox_labels",
  "live_videos",
  "standby",
  "messaging_handovers",
  "leadgen",
]
const MARKETING_FIELDS = ["marketing_message_echoes", "marketing_message_deliveries", "marketing_message_delivery_failed"]

type Automation = {
  id: string
  name: string
  pageId: string
  pageName: string | null
  formId: string
  formName: string | null
  fieldMapping: Array<{ key: string; label: string; type: string; target: string | null }>
  leadsHandledCount: number
  createdAt: string
  updatedAt: string
}

const graphError = (message: string, code = 100) => ({
  status: 400,
  json: { error: { message, type: "OAuthException", code, fbtrace_id: "parity" } },
})

const subscribeOk = (times = 1) =>
  fake.script({
    match: { host: GRAPH, method: "POST", pathRegex: graphPath("me/subscribed_apps") },
    respond: { status: 200, json: { success: true } },
    times,
  })

const subscribeCalls = async (since: number, accessToken: string) =>
  (await upstreamCalls({ host: GRAPH, since, method: "POST", path: /^\/v[0-9.]+\/me\/subscribed_apps$/ })).filter(
    (record) => record.headers.authorization === `Bearer ${accessToken}`,
  )

const EMAIL_MAPPING = { key: "email", label: "Email", type: "EMAIL", target: "email" }

const createAutomation = async (api: Api, body: Record<string, unknown>): Promise<Automation> => {
  await subscribeOk()
  const response = await api.post<Automation>("/v1/facebook-lead-ads", body)
  expect(response.status, response.text).toBe(201)
  return response.json
}

describe("facebookLeadAds.create", () => {
  it("subscribes the page to leadgen (with marketing message fields) before saving the automation", async () => {
    const { tenant, api } = await tenantApi("p108-la-create")
    const page = await messengerPage(tenant.workspaceId)
    const name = uniqueName("Leads")
    await subscribeOk()
    const since = await fake.cursor()
    const response = await api.post<Automation>("/v1/facebook-lead-ads", {
      name,
      pageId: page.pageId,
      pageName: page.name,
      formId: "777",
      formName: "Spring form",
      fieldMapping: [EMAIL_MAPPING, { key: "city", label: "City", type: "CUSTOM", target: null }],
    })
    expect(response.status, response.text).toBe(201)
    expect(keysOf(response.json)).toEqual(AUTOMATION_KEYS)
    expect(response.json).toMatchObject({
      name,
      pageId: page.pageId,
      pageName: page.name,
      formId: "777",
      formName: "Spring form",
      fieldMapping: [EMAIL_MAPPING, { key: "city", label: "City", type: "CUSTOM", target: null }],
      leadsHandledCount: 0,
    })
    expect(response.json.id).toMatch(/^\d{15,20}$/)

    const calls = await subscribeCalls(since, page.accessToken)
    expect(calls).toHaveLength(1)
    expect(calls[0]?.path).toBe("/v23.0/me/subscribed_apps")
    expect(JSON.parse(calls[0]?.bodyText ?? "{}")).toEqual({
      subscribed_fields: [...LEAD_FIELDS, ...MARKETING_FIELDS].join(","),
    })

    const got = await api.get<Automation>(`/v1/facebook-lead-ads/${response.json.id}`)
    expect(got.status).toBe(200)
    expect(got.json).toEqual(response.json)
  })

  it("retries the subscription without the marketing message fields when Graph refuses them", async () => {
    const { tenant, api } = await tenantApi("p108-la-create-retry")
    const page = await messengerPage(tenant.workspaceId)
    await subscribeOk()
    await fake.script({
      match: { host: GRAPH, method: "POST", pathRegex: graphPath("me/subscribed_apps"), bodyIncludes: "marketing_message_echoes" },
      respond: graphError("(#200) Marketing messages not enabled"),
      times: 1,
    })
    const since = await fake.cursor()
    const response = await api.post<Automation>("/v1/facebook-lead-ads", {
      name: uniqueName("Retry"),
      pageId: page.pageId,
      formId: "*",
    })
    expect(response.status, response.text).toBe(201)
    expect(response.json).toMatchObject({ formId: "*", pageName: null, formName: null, fieldMapping: [] })
    const calls = await subscribeCalls(since, page.accessToken)
    expect(calls.map((call) => JSON.parse(call.bodyText).subscribed_fields)).toEqual([
      [...LEAD_FIELDS, ...MARKETING_FIELDS].join(","),
      LEAD_FIELDS.join(","),
    ])
  })

  it("refuses when the page cannot be subscribed (no Messenger channel, or Graph refuses): 400, nothing saved", async () => {
    const { tenant, api } = await tenantApi("p108-la-create-fail")
    const since = await fake.cursor()
    expectError(
      await api.post("/v1/facebook-lead-ads", { name: "No page", pageId: uniqueDigits(), formId: "1" }),
      400,
      "systemError",
      { defined: true, message: "Failed to subscribe the page to lead webhooks. Try again." },
    )
    expect(await upstreamCalls({ host: GRAPH, since, path: /subscribed_apps/ })).toHaveLength(0)

    const page = await messengerPage(tenant.workspaceId)
    await fake.script({
      match: { host: GRAPH, method: "POST", pathRegex: graphPath("me/subscribed_apps") },
      respond: graphError("(#190) Invalid OAuth access token", 190),
      times: 2,
    })
    expectError(
      await api.post("/v1/facebook-lead-ads", { name: "Refused", pageId: page.pageId, formId: "1" }),
      400,
      "systemError",
      { message: "Failed to subscribe the page to lead webhooks. Try again." },
    )
    expect(await subscribeCalls(since, page.accessToken)).toHaveLength(2)
    expect((await api.get("/v1/facebook-lead-ads")).json).toEqual({ data: [], pageCount: 0 })
  })

  it("refuses a second automation for the same page and form (422 validation) but allows another form", async () => {
    const { tenant, api } = await tenantApi("p108-la-create-dup")
    const page = await messengerPage(tenant.workspaceId)
    await createAutomation(api, { name: "First", pageId: page.pageId, formId: "42" })
    await subscribeOk()
    expectError(await api.post("/v1/facebook-lead-ads", { name: "Second", pageId: page.pageId, formId: "42" }), 422, "validation", {
      defined: true,
      message: "An automation for this page and form already exists.",
    })
    await createAutomation(api, { name: "All forms", pageId: page.pageId, formId: "*" })
  })

  it("validates the input (422)", async () => {
    const { api } = await tenantApi("p108-la-create-invalid")
    expectInvalid(await api.post("/v1/facebook-lead-ads", { pageId: "1", formId: "1" }), "name")
    expectInvalid(await api.post("/v1/facebook-lead-ads", { name: "x".repeat(101), pageId: "1", formId: "1" }), "name")
    expectInvalid(await api.post("/v1/facebook-lead-ads", { name: "n", pageId: "", formId: "1" }), "pageId")
    expectInvalid(await api.post("/v1/facebook-lead-ads", { name: "n", pageId: "1" }), "formId")
    expectInvalid(
      await api.post("/v1/facebook-lead-ads", { name: "n", pageId: "1", formId: "1", fieldMapping: [{ key: "email" }] }),
      "fieldMapping.0.label",
    )
  })
})

describe("facebookLeadAds.get / list", () => {
  it("returns 404 for an unknown or foreign automation and 422 for a non-numeric id", async () => {
    const { api } = await tenantApi("p108-la-get")
    const { tenant: other, api: otherApi } = await tenantApi("p108-la-get-other")
    const page = await messengerPage(other.workspaceId)
    const foreign = await createAutomation(otherApi, { name: "Theirs", pageId: page.pageId, formId: "1" })
    expectError(await api.get("/v1/facebook-lead-ads/1"), 404, "notFound", {
      defined: true,
      message: "Facebook Lead Ads automation not found",
    })
    expectError(await api.get(`/v1/facebook-lead-ads/${foreign.id}`), 404, "notFound")
    expectInvalid(await api.get("/v1/facebook-lead-ads/abc"), "id")
  })

  it("lists automations, filters by keyword on the name, sorts and pages", async () => {
    const { tenant, api } = await tenantApi("p108-la-list")
    const page = await messengerPage(tenant.workspaceId)
    const a = await createAutomation(api, { name: "alpha leads", pageId: page.pageId, formId: "1" })
    const b = await createAutomation(api, { name: "beta leads", pageId: page.pageId, formId: "2" })
    const c = await createAutomation(api, { name: "gamma ALPHA", pageId: page.pageId, formId: "3" })

    const sorted = await api.get<{ data: Automation[]; pageCount: number }>(
      `/v1/facebook-lead-ads?${sortQuery([{ id: "name", desc: false }])}`,
    )
    expect(sorted.status, sorted.text).toBe(200)
    expect(sorted.json).toEqual({ data: [a, b, c], pageCount: 1 })

    const filtered = await api.get<{ data: Automation[] }>(
      `/v1/facebook-lead-ads?${sortQuery([{ id: "name", desc: true }])}&${qs({ keyword: "Alpha" }).slice(1)}`,
    )
    expect(filtered.json.data.map((row) => row.id)).toEqual([c.id, a.id])

    const page2 = await api.get<{ data: Automation[]; pageCount: number }>(
      `/v1/facebook-lead-ads?${sortQuery([{ id: "name", desc: false }])}&perPage=2&page=2`,
    )
    expect(page2.json).toEqual({ data: [c], pageCount: 2 })
    expect((await api.get(`/v1/facebook-lead-ads${qs({ keyword: "none-such" })}`)).json).toEqual({ data: [], pageCount: 0 })
  })
})

describe("facebookLeadAds.update", () => {
  it("changes the name and field mapping only; 404 for an unknown id", async () => {
    const { tenant, api } = await tenantApi("p108-la-update")
    const page = await messengerPage(tenant.workspaceId)
    const created = await createAutomation(api, { name: "Before", pageId: page.pageId, formId: "9", fieldMapping: [EMAIL_MAPPING] })
    const since = await fake.cursor()

    const renamed = await api.patch<Automation>(`/v1/facebook-lead-ads/${created.id}`, { name: "After", pageId: "999", formId: "1" })
    expect(renamed.status, renamed.text).toBe(200)
    expect(keysOf(renamed.json)).toEqual(AUTOMATION_KEYS)
    expect(renamed.json).toMatchObject({ ...created, name: "After", updatedAt: expect.any(String) })

    const remapped = await api.patch<Automation>(`/v1/facebook-lead-ads/${created.id}`, {
      fieldMapping: [{ key: "phone", label: "Phone", type: "PHONE", target: "phone" }],
    })
    expect(remapped.json).toMatchObject({ name: "After", formId: "9", fieldMapping: [{ key: "phone", label: "Phone", type: "PHONE", target: "phone" }] })
    expect((await api.get(`/v1/facebook-lead-ads/${created.id}`)).json).toEqual(remapped.json)
    // No Graph call on update.
    expect(await upstreamCalls({ host: GRAPH, since })).toHaveLength(0)

    expectError(await api.patch("/v1/facebook-lead-ads/1", { name: "x" }), 404, "notFound", {
      defined: true,
      message: "Facebook Lead Ads automation not found",
    })
    expectInvalid(await api.patch(`/v1/facebook-lead-ads/${created.id}`, { name: "" }), "name")
  })
})

describe("facebookLeadAds.delete", () => {
  it("deletes an automation (204); an unknown or foreign id is also 204 and deletes nothing", async () => {
    const { tenant, api } = await tenantApi("p108-la-delete")
    const { tenant: other, api: otherApi } = await tenantApi("p108-la-delete-other")
    const page = await messengerPage(tenant.workspaceId)
    const otherPage = await messengerPage(other.workspaceId)
    const mine = await createAutomation(api, { name: "Mine", pageId: page.pageId, formId: "1" })
    const foreign = await createAutomation(otherApi, { name: "Theirs", pageId: otherPage.pageId, formId: "1" })

    const deleted = await api.delete(`/v1/facebook-lead-ads/${mine.id}`)
    expect(deleted.status).toBe(204)
    expectError(await api.get(`/v1/facebook-lead-ads/${mine.id}`), 404, "notFound")
    expect((await api.delete(`/v1/facebook-lead-ads/${mine.id}`)).status).toBe(204)
    expect((await api.delete(`/v1/facebook-lead-ads/${foreign.id}`)).status).toBe(204)
    expect((await otherApi.get(`/v1/facebook-lead-ads/${foreign.id}`)).status).toBe(200)
    expectInvalid(await api.delete("/v1/facebook-lead-ads/abc"), "id")
  })
})

const debugTokenScript = (data: Record<string, unknown>) =>
  fake.script({
    match: { host: GRAPH, method: "GET", pathRegex: graphPath("debug_token") },
    respond: { status: 200, json: { data } },
    times: 1,
  })

describe("facebookLeadAds.listPages", () => {
  it("flags a page eligible when its token is valid and carries leads_retrieval (debug_token with the app token)", async () => {
    const { tenant, api } = await tenantApi("p108-la-pages")
    const page = await messengerPage(tenant.workspaceId, "Eligible page")
    await debugTokenScript({ is_valid: true, scopes: ["pages_messaging", "leads_retrieval"] })
    const since = await fake.cursor()
    const response = await api.get("/v1/facebook-lead-ads/pages")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ pages: [{ pageId: page.pageId, pageName: "Eligible page", eligible: true }] })
    const calls = await upstreamCalls({ host: GRAPH, since, path: /^\/v[0-9.]+\/debug_token$/ })
    expect(calls).toHaveLength(1)
    expect(calls[0]).toMatchObject({ method: "GET", path: "/v23.0/debug_token", query: { input_token: page.accessToken } })
    expect(calls[0]?.headers.authorization).toBe("Bearer parity-client|parity-secret")
  })

  it("flags a page ineligible without leads_retrieval, with an invalid token, or without the app credentials", async () => {
    const { tenant, api } = await tenantApi("p108-la-pages-no")
    const page = await messengerPage(tenant.workspaceId, "No scope")
    await debugTokenScript({ is_valid: true, scopes: ["pages_messaging"] })
    expect((await api.get("/v1/facebook-lead-ads/pages")).json).toEqual({
      pages: [{ pageId: page.pageId, pageName: "No scope", eligible: false }],
    })
    await debugTokenScript({ is_valid: false, scopes: ["leads_retrieval"] })
    expect((await api.get("/v1/facebook-lead-ads/pages")).json).toEqual({
      pages: [{ pageId: page.pageId, pageName: "No scope", eligible: false }],
    })

    const { tenant: bare, api: bareApi } = await tenantApi("p108-la-pages-bare")
    const pageId = uniqueDigits()
    await (await ext()).createMessengerChannel({ workspaceId: bare.workspaceId, pageId, name: "Bare", withAppCredentials: false })
    const since = await fake.cursor()
    expect((await bareApi.get("/v1/facebook-lead-ads/pages")).json).toEqual({ pages: [{ pageId, pageName: "Bare", eligible: false }] })
    expect(await upstreamCalls({ host: GRAPH, since, path: /debug_token/ })).toHaveLength(0)
  })

  it("drops a page whose debug_token call fails; an empty workspace has no pages", async () => {
    const { tenant, api } = await tenantApi("p108-la-pages-fail")
    await messengerPage(tenant.workspaceId)
    await fake.script({
      match: { host: GRAPH, method: "GET", pathRegex: graphPath("debug_token") },
      respond: graphError("(#100) You must provide an app access token"),
      times: 1,
    })
    expect((await api.get("/v1/facebook-lead-ads/pages")).json).toEqual({ pages: [] })
    const { api: empty } = await tenantApi("p108-la-pages-empty")
    expect((await empty.get("/v1/facebook-lead-ads/pages")).json).toEqual({ pages: [] })
  })
})

describe("facebookLeadAds.listForms", () => {
  it("lists the page's lead forms from Graph with the page token", async () => {
    const { tenant, api } = await tenantApi("p108-la-forms")
    const page = await messengerPage(tenant.workspaceId)
    const forms = [
      {
        id: "1001",
        name: "Spring",
        status: "ACTIVE",
        questions: [{ key: "email", label: "Email", type: "EMAIL", id: "q1" }],
        locale: "en_US",
      },
      { id: "1002", name: "Old", status: "ARCHIVED" },
    ]
    await fake.script({
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${reEscape(page.pageId)}/leadgen_forms`) },
      respond: { status: 200, json: { data: forms } },
    })
    const since = await fake.cursor()
    const response = await api.get(`/v1/facebook-lead-ads/forms${qs({ pageId: page.pageId })}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      forms: [
        { id: "1001", name: "Spring", status: "ACTIVE", questions: [{ key: "email", label: "Email", type: "EMAIL", id: "q1" }] },
        { id: "1002", name: "Old", status: "ARCHIVED" },
      ],
    })
    const calls = await upstreamCalls({ host: GRAPH, since, path: new RegExp(`/${page.pageId}/leadgen_forms$`) })
    expect(calls).toHaveLength(1)
    expect(calls[0]).toMatchObject({
      method: "GET",
      path: `/v23.0/${page.pageId}/leadgen_forms`,
      query: { fields: "id,name,status,questions", limit: "100", access_token: page.accessToken },
    })
  })

  it("returns no forms for a page that is not connected; requires pageId; surfaces a Graph error", async () => {
    const { tenant, api } = await tenantApi("p108-la-forms-errors")
    const since = await fake.cursor()
    expect((await api.get(`/v1/facebook-lead-ads/forms${qs({ pageId: uniqueDigits() })}`)).json).toEqual({ forms: [] })
    expect(await upstreamCalls({ host: GRAPH, since })).toHaveLength(0)
    expectInvalid(await api.get("/v1/facebook-lead-ads/forms"), "pageId")

    const page = await messengerPage(tenant.workspaceId)
    await fake.script({
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${reEscape(page.pageId)}/leadgen_forms`) },
      respond: graphError("(#200) Requires leads_retrieval permission", 200),
    })
    const failed = await api.get(`/v1/facebook-lead-ads/forms${qs({ pageId: page.pageId })}`)
    const body = expectError(failed, 400, "BAD_REQUEST", { defined: false })
    expect(body.message).toContain("Requires leads_retrieval permission")
  })
})
