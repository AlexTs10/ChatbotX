// ads.* conversion rules: listRules, getRule, createRule, updateRule,
// updateRuleLegacy, toggleRuleStatus, deleteRule.

import { describe, expect, it } from "vitest"
import { createTenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { Normalizer } from "../../src/normalize.ts"
import { adsFixtures, expectError, expectValidationIssue, MISSING_ID, uniqueDigits } from "./ads-helpers.ts"

type Rule = Record<string, unknown> & { id: string }

const setup = async (label: string) => {
  const tenant = await createTenant({ label })
  const ext = await adsFixtures()
  const pageId = uniqueDigits()
  const messenger = await ext.createMessengerChannel({ workspaceId: tenant.workspaceId, pageId, name: `Page ${pageId}` })
  const phoneNumberId = uniqueDigits()
  const whatsapp = await ext.createWhatsappChannel({
    workspaceId: tenant.workspaceId,
    phoneNumberId,
    wabaId: `waba-${phoneNumberId}`,
    name: `WA ${phoneNumberId}`,
    displayPhoneNumber: "+15550001111",
  })
  return { tenant, api: apiClient(tenant.token), messenger, whatsapp }
}

const messengerRule = (integrationMessengerId: string, extra: Record<string, unknown> = {}) => ({
  channel: "messenger",
  integrationMessengerId,
  eventType: "lead",
  trigger: { type: "tagApplied", tagIds: ["tag-a", "tag-b"] },
  markAs: "Lead from tag",
  ...extra,
})

describe("ads.createRule (POST /v1/ads/conversion-rules)", () => {
  it("creates a rule and returns the full resource without workspaceId", async () => {
    const normalizer = new Normalizer()
    const { api, messenger } = await setup("rule-create")
    const response = await api.post<Rule>("/v1/ads/conversion-rules", messengerRule(messenger.integrationId, { adAccountId: "act_42" }))
    expect(response.status, response.text).toBe(201)
    expect(normalizer.normalize(response.json, { replace: { [messenger.integrationId]: "<messenger>" } })).toEqual({
      id: "<id:1>",
      channel: "messenger",
      integrationWhatsappId: null,
      integrationFacebookAdsId: null,
      integrationMessengerId: "<messenger>",
      integrationInstagramId: null,
      adAccountId: "act_42",
      eventType: "lead",
      trigger: { type: "tagApplied", tagIds: ["tag-a", "tag-b"] },
      markAs: "Lead from tag",
      enabled: true,
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })
  })

  it("defaults adAccountId/markAs to null and keeps enabled=false", async () => {
    const { api, whatsapp } = await setup("rule-create-wa")
    const response = await api.post<Rule>("/v1/ads/conversion-rules", {
      channel: "whatsapp",
      integrationWhatsappId: whatsapp.integrationId,
      eventType: "purchase",
      trigger: { type: "templateSent", templateIds: ["tpl-1"] },
      enabled: false,
    })
    expect(response.status, response.text).toBe(201)
    expect(response.json).toMatchObject({
      channel: "whatsapp",
      integrationWhatsappId: whatsapp.integrationId,
      integrationMessengerId: null,
      adAccountId: null,
      markAs: null,
      eventType: "purchase",
      trigger: { type: "templateSent", templateIds: ["tpl-1"] },
      enabled: false,
    })
    expect(response.json).not.toHaveProperty("workspaceId")
  })

  it("rejects an invalid body with 422 invalidRequestData", async () => {
    const { api, messenger } = await setup("rule-create-422")
    expectValidationIssue(
      await api.post("/v1/ads/conversion-rules", messengerRule(messenger.integrationId, { trigger: { type: "tagApplied", tagIds: [] } })),
      ["trigger", "tagIds"],
    )
    expectValidationIssue(
      await api.post("/v1/ads/conversion-rules", messengerRule(messenger.integrationId, { eventType: "signup" })),
      ["eventType"],
    )
    expectValidationIssue(
      await api.post("/v1/ads/conversion-rules", messengerRule(messenger.integrationId, { channel: "telegram" })),
      ["channel"],
    )
    expectValidationIssue(
      await api.post("/v1/ads/conversion-rules", { channel: "messenger", eventType: "lead" }),
      ["trigger"],
    )
  })

  it("rejects an integration of another channel with 400 systemError", async () => {
    const { api, messenger } = await setup("rule-create-mismatch")
    expectError(
      await api.post("/v1/ads/conversion-rules", {
        ...messengerRule(messenger.integrationId),
        channel: "whatsapp",
        integrationMessengerId: undefined,
        integrationWhatsappId: undefined,
      }),
      400,
      "systemError",
      "Ads conversion rule integration must match the selected channel",
    )
  })

  it("rejects an integration of another workspace with 400 systemError", async () => {
    const { api } = await setup("rule-create-foreign")
    const other = await setup("rule-create-foreign-2")
    expectError(
      await api.post("/v1/ads/conversion-rules", messengerRule(other.messenger.integrationId)),
      400,
      "systemError",
      "Ads conversion rule integration was not found in this workspace",
    )
  })

  it("rejects templateSent on instagram with 400 systemError", async () => {
    const tenant = await createTenant({ label: "rule-ig" })
    const ext = await adsFixtures()
    const igId = uniqueDigits()
    const instagram = await ext.createInstagramChannel({
      workspaceId: tenant.workspaceId,
      igId,
      pageId: uniqueDigits(),
      name: `IG ${igId}`,
      username: `ig_${igId}`,
    })
    expectError(
      await apiClient(tenant.token).post("/v1/ads/conversion-rules", {
        channel: "instagram",
        integrationInstagramId: instagram.integrationId,
        eventType: "lead",
        trigger: { type: "templateSent", templateIds: ["tpl-1"] },
      }),
      400,
      "systemError",
      'Ads conversion trigger type "templateSent" is not supported for channel "instagram"',
    )
  })
})

describe("ads.listRules (GET /v1/ads/conversion-rules)", () => {
  it("lists rules oldest first with in-memory paging and a channel filter", async () => {
    const { api, messenger, whatsapp } = await setup("rule-list")
    const empty = await api.get("/v1/ads/conversion-rules")
    expect(empty.status, empty.text).toBe(200)
    expect(empty.json).toEqual({ data: [], pageCount: 1 })

    const first = await api.post<Rule>("/v1/ads/conversion-rules", messengerRule(messenger.integrationId))
    const second = await api.post<Rule>("/v1/ads/conversion-rules", {
      channel: "whatsapp",
      integrationWhatsappId: whatsapp.integrationId,
      eventType: "purchase",
      trigger: { type: "contactReplied", firstReplyOnly: true },
    })
    const third = await api.post<Rule>(
      "/v1/ads/conversion-rules",
      messengerRule(messenger.integrationId, { trigger: { type: "keywordMatched", automatedResponseIds: ["kw-1"] } }),
    )
    expect([first.status, second.status, third.status]).toEqual([201, 201, 201])

    const all = await api.get<{ data: Rule[]; pageCount: number }>("/v1/ads/conversion-rules")
    expect(all.status).toBe(200)
    expect(all.json.pageCount).toBe(1)
    expect(all.json.data.map((rule) => rule.id)).toEqual([first.json.id, second.json.id, third.json.id])
    expect(all.json.data[1]).toEqual(second.json)

    const page2 = await api.get<{ data: Rule[]; pageCount: number }>("/v1/ads/conversion-rules", {
      query: { page: 2, perPage: 2 },
    })
    expect(page2.status).toBe(200)
    expect(page2.json).toEqual({ data: [third.json], pageCount: 2 })

    const page9 = await api.get("/v1/ads/conversion-rules", { query: { page: 9, perPage: 2 } })
    expect(page9.json).toEqual({ data: [], pageCount: 2 })

    const messengerOnly = await api.get<{ data: Rule[] }>("/v1/ads/conversion-rules", { query: { channel: "messenger" } })
    expect(messengerOnly.json.data.map((rule) => rule.id)).toEqual([first.json.id, third.json.id])
  })

  it("rejects bad paging and channel values with 422", async () => {
    const { api } = await setup("rule-list-422")
    expectValidationIssue(await api.get("/v1/ads/conversion-rules", { query: { perPage: 51 } }), ["perPage"])
    expectValidationIssue(await api.get("/v1/ads/conversion-rules", { query: { page: 0 } }), ["page"])
    expectValidationIssue(await api.get("/v1/ads/conversion-rules", { query: { channel: "telegram" } }), ["channel"])
  })
})

describe("ads.getRule (GET /v1/ads/conversion-rules/{id})", () => {
  it("returns one rule; 404 for a missing or foreign id; 422 for a non-numeric id", async () => {
    const { api, messenger } = await setup("rule-get")
    const created = await api.post<Rule>("/v1/ads/conversion-rules", messengerRule(messenger.integrationId))
    const got = await api.get(`/v1/ads/conversion-rules/${created.json.id}`)
    expect(got.status, got.text).toBe(200)
    expect(got.json).toEqual(created.json)

    expectError(await api.get(`/v1/ads/conversion-rules/${MISSING_ID}`), 404, "notFound", "Ads conversion rule not found")
    const other = await setup("rule-get-other")
    expectError(await other.api.get(`/v1/ads/conversion-rules/${created.json.id}`), 404, "notFound")
    expectValidationIssue(await api.get("/v1/ads/conversion-rules/abc"), ["id"])
  })
})

describe("ads.updateRule (PATCH /v1/ads/conversion-rules/{id})", () => {
  it("changes only the given fields", async () => {
    const { api, messenger } = await setup("rule-patch")
    const created = await api.post<Rule>("/v1/ads/conversion-rules", messengerRule(messenger.integrationId))
    const updated = await api.patch<Rule>(`/v1/ads/conversion-rules/${created.json.id}`, {
      eventType: "purchase",
      markAs: null,
      trigger: { type: "contactReplied", firstReplyOnly: false },
    })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).toEqual({
      ...created.json,
      eventType: "purchase",
      markAs: null,
      trigger: { type: "contactReplied", firstReplyOnly: false },
      updatedAt: expect.any(String),
    })
    expect(Date.parse(updated.json.updatedAt as string)).toBeGreaterThanOrEqual(Date.parse(created.json.updatedAt as string))

    const toggled = await api.patch<Rule>(`/v1/ads/conversion-rules/${created.json.id}`, { enabled: false })
    expect(toggled.status).toBe(200)
    expect(toggled.json).toMatchObject({ enabled: false, eventType: "purchase", markAs: null })
    const got = await api.get(`/v1/ads/conversion-rules/${created.json.id}`)
    expect(got.json).toEqual(toggled.json)
  })

  it("rejects a channel change without a matching integration (400) and a missing rule (404)", async () => {
    const { api, messenger } = await setup("rule-patch-err")
    const created = await api.post<Rule>("/v1/ads/conversion-rules", messengerRule(messenger.integrationId))
    expectError(
      await api.patch(`/v1/ads/conversion-rules/${created.json.id}`, { channel: "whatsapp" }),
      400,
      "systemError",
      "Ads conversion rule integration must match the selected channel",
    )
    expectError(await api.patch(`/v1/ads/conversion-rules/${MISSING_ID}`, { enabled: false }), 404, "notFound")
    expectValidationIssue(await api.patch(`/v1/ads/conversion-rules/${created.json.id}`, { enabled: "no" }), ["enabled"])
  })
})

describe("ads.updateRuleLegacy (PUT /v1/ads/conversion-rules/{id})", () => {
  it("behaves like PATCH: omitted fields stay unchanged", async () => {
    const { api, messenger } = await setup("rule-put")
    const created = await api.post<Rule>("/v1/ads/conversion-rules", messengerRule(messenger.integrationId))
    const updated = await api.put<Rule>(`/v1/ads/conversion-rules/${created.json.id}`, { markAs: "Renamed" })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).toEqual({ ...created.json, markAs: "Renamed", updatedAt: expect.any(String) })
    expectError(await api.put(`/v1/ads/conversion-rules/${MISSING_ID}`, { markAs: "x" }), 404, "notFound")
  })
})

describe("ads.toggleRuleStatus (PATCH /v1/ads/conversion-rules/{id}/status)", () => {
  it("sets enabled; requires enabled; 404 for a missing rule", async () => {
    const { api, messenger } = await setup("rule-status")
    const created = await api.post<Rule>("/v1/ads/conversion-rules", messengerRule(messenger.integrationId))
    const off = await api.patch<Rule>(`/v1/ads/conversion-rules/${created.json.id}/status`, { enabled: false })
    expect(off.status, off.text).toBe(200)
    expect(off.json).toEqual({ ...created.json, enabled: false, updatedAt: expect.any(String) })
    const on = await api.patch<Rule>(`/v1/ads/conversion-rules/${created.json.id}/status`, { enabled: true })
    expect(on.json.enabled).toBe(true)
    expectValidationIssue(await api.patch(`/v1/ads/conversion-rules/${created.json.id}/status`, {}), ["enabled"])
    expectError(
      await api.patch(`/v1/ads/conversion-rules/${MISSING_ID}/status`, { enabled: true }),
      404,
      "notFound",
      "Ads conversion rule not found",
    )
  })
})

describe("ads.deleteRule (DELETE /v1/ads/conversion-rules/{id})", () => {
  it("deletes with 204 and an empty body; then 404", async () => {
    const { api, messenger } = await setup("rule-delete")
    const created = await api.post<Rule>("/v1/ads/conversion-rules", messengerRule(messenger.integrationId))
    const other = await setup("rule-delete-other")
    expectError(await other.api.delete(`/v1/ads/conversion-rules/${created.json.id}`), 404, "notFound")

    const deleted = await api.delete(`/v1/ads/conversion-rules/${created.json.id}`)
    expect(deleted.status, deleted.text).toBe(204)
    expect(deleted.text).toBe("")
    expectError(await api.get(`/v1/ads/conversion-rules/${created.json.id}`), 404, "notFound")
    expectError(await api.delete(`/v1/ads/conversion-rules/${created.json.id}`), 404, "notFound", "Ads conversion rule not found")
    const list = await api.get("/v1/ads/conversion-rules")
    expect(list.json).toEqual({ data: [], pageCount: 1 })
  })
})
