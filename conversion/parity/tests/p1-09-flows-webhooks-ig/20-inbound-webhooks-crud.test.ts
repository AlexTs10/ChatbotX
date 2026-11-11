// inboundWebhooks.create / get / list / update / updateSettings /
// regenerateToken / delete / deleteMany: resource shapes, configuration
// validation (zod and business rules), the endpoint URL and its secret.
// The auth matrix of every operation is in 00-auth.test.ts.

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { apiClient } from "../../src/http.ts"
import {
  type Api,
  configure,
  createApiChannel,
  createCustomField,
  createFlow,
  createTag,
  createWebhook,
  deliver,
  ENDPOINT_URL,
  expectError,
  expectInvalid,
  expectIso,
  expectValidation,
  extraToken,
  putConfig,
  RESOURCE_KEYS,
  SNOWFLAKE,
  SUMMARY_KEYS,
  setActive,
  tenantApi,
  uniqueName,
  type Webhook,
  type WebhookConfig,
  type WebhookSummary,
} from "./helpers-webhooks.ts"

const BASE = "/v1/inbound-webhooks"

/** A complete configuration on an API-channel inbox. */
const apiConfig = (inboxId: string, overrides: Partial<WebhookConfig> = {}): WebhookConfig => ({
  inboxResolution: { mode: "inbox", inboxId },
  identifiers: [{ kind: "channelUserId", jsonPath: "$.contact.id" }],
  fieldMappings: [{ jsonPath: "$.contact.first", target: "first_name" }],
  actions: [],
  ...overrides,
})

describe("inboundWebhooks.create", () => {
  it("creates an inactive, unconfigured webhook with a secret endpoint URL (201)", async () => {
    const start = Date.now()
    const { api } = await tenantApi("p109-iwh-create")
    const name = uniqueName("Orders")
    const response = await api.post<Webhook>(BASE, { name: `  ${name}  ` })
    expect(response.status, response.text).toBe(201)
    const body = response.json
    expect(Object.keys(body).sort()).toEqual(RESOURCE_KEYS)
    expect(body).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      name,
      active: false,
      createdAt: expect.any(String),
      updatedAt: expect.any(String),
      requestCount: 0,
      endpointUrl: expect.stringMatching(ENDPOINT_URL),
      inboxResolution: null,
      identifiers: [],
      fieldMappings: [],
      actions: [],
      sampleJson: null,
    })
    expectIso(body.createdAt, start)
    expectIso(body.updatedAt, start)
  })

  it("gives each webhook its own token", async () => {
    const { api } = await tenantApi("p109-iwh-create-2")
    const first = await createWebhook(api)
    const second = await createWebhook(api)
    expect(first.id).not.toBe(second.id)
    expect(first.endpointUrl).not.toBe(second.endpointUrl)
  })

  it("validates the name (trimmed, 1-255 characters)", async () => {
    const { api } = await tenantApi("p109-iwh-create-bad")
    expectInvalid(await api.post(BASE, {}), "name")
    expectInvalid(await api.post(BASE, { name: "   " }), "name")
    expectInvalid(await api.post(BASE, { name: "x".repeat(256) }), "name")
    expectInvalid(await api.post(BASE, { name: 42 }), "name")
    const max = await api.post<Webhook>(BASE, { name: "x".repeat(255) })
    expect(max.status, max.text).toBe(201)
  })

  it(
    "allows 100 webhooks per workspace, then 422 validation maxItemsReached; list pages newest first",
    async () => {
      const { api } = await tenantApi("p109-iwh-limit")
      const ids: string[] = []
      for (let index = 0; index < 100; index += 1) {
        ids.push((await createWebhook(api, `limit-${String(index).padStart(3, "0")}`)).id)
      }
      expectValidation(
        await api.post(BASE, { name: "one too many" }),
        "validation.maxItemsReached (max=100, feature=inboundWebhooks)",
      )

      const page1 = await api.get<{ data: WebhookSummary[]; pageCount: number }>(`${BASE}?perPage=50`)
      expect(page1.status, page1.text).toBe(200)
      expect(page1.json.pageCount).toBe(2)
      expect(page1.json.data).toHaveLength(50)
      const page2 = await api.get<{ data: WebhookSummary[]; pageCount: number }>(`${BASE}?perPage=50&page=2`)
      expect(page2.json.pageCount).toBe(2)
      // Newest first (createdAt desc, then id desc): the last created comes first.
      expect([...page1.json.data, ...page2.json.data].map((item) => item.id)).toEqual([...ids].reverse())
      const page3 = await api.get<{ data: WebhookSummary[]; pageCount: number }>(`${BASE}?perPage=50&page=3`)
      expect(page3.json).toEqual({ data: [], pageCount: 2 })

      // A deleted one frees a slot.
      expect((await api.delete(`${BASE}/${ids[0]}`)).status).toBe(204)
      expect((await api.post(BASE, { name: "fits again" })).status).toBe(201)
    },
    120_000,
  )
})

describe("inboundWebhooks.list", () => {
  it("lists summaries only (no endpoint URL, config or sample), newest first", async () => {
    const start = Date.now()
    const { api } = await tenantApi("p109-iwh-list")
    const empty = await api.get<{ data: WebhookSummary[]; pageCount: number }>(BASE)
    expect(empty.status).toBe(200)
    expect(empty.json).toEqual({ data: [], pageCount: 0 })

    const older = await createWebhook(api, "older")
    const newer = await createWebhook(api, "newer")
    const list = await api.get<{ data: WebhookSummary[]; pageCount: number }>(BASE)
    expect(list.status, list.text).toBe(200)
    expect(list.json.pageCount).toBe(1)
    expect(list.json.data.map((item) => item.id)).toEqual([newer.id, older.id])
    for (const item of list.json.data) {
      expect(Object.keys(item).sort()).toEqual(SUMMARY_KEYS)
      expectIso(item.createdAt, start)
      expectIso(item.updatedAt, start)
    }
    expect(list.json.data[1]).toEqual({
      id: older.id,
      name: "older",
      active: false,
      createdAt: older.createdAt,
      updatedAt: older.updatedAt,
      requestCount: 0,
    })
    expect(list.text).not.toContain("iwh_")
  })

  it("pages with page/perPage and validates them", async () => {
    const { api } = await tenantApi("p109-iwh-list-page")
    const a = await createWebhook(api, "a")
    const b = await createWebhook(api, "b")
    const c = await createWebhook(api, "c")
    const page = (query: string) => api.get<{ data: WebhookSummary[]; pageCount: number }>(`${BASE}?${query}`)
    const first = await page("perPage=2")
    expect({ ids: first.json.data.map((x) => x.id), pageCount: first.json.pageCount }).toEqual({
      ids: [c.id, b.id],
      pageCount: 2,
    })
    const second = await page("perPage=2&page=2")
    expect(second.json.data.map((x) => x.id)).toEqual([a.id])
    expectInvalid(await page("perPage=0"), "perPage")
    expectInvalid(await page("page=0"), "page")
    expectInvalid(await page("perPage=abc"), "perPage")
  })

  it("never shows another workspace's webhooks", async () => {
    const { api } = await tenantApi("p109-iwh-list-own")
    const other = await tenantApi("p109-iwh-list-other")
    await createWebhook(other.api)
    const list = await api.get<{ data: WebhookSummary[] }>(BASE)
    expect(list.json.data).toEqual([])
  })
})

describe("inboundWebhooks.get", () => {
  it("returns the full resource with the endpoint URL; a read-only token gets no URL", async () => {
    const { tenant, api } = await tenantApi("p109-iwh-get")
    const created = await createWebhook(api)
    const response = await api.get<Webhook>(`${BASE}/${created.id}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual(created)

    const readOnly = apiClient(await extraToken(tenant, { permission: "read_only" }))
    const masked = await readOnly.get<Webhook>(`${BASE}/${created.id}`)
    expect(masked.status, masked.text).toBe(200)
    const { endpointUrl: _url, ...withoutUrl } = created
    expect(masked.json).toEqual(withoutUrl)
    expect("endpointUrl" in masked.json).toBe(false)
  })

  it("404 notFound for an unknown id or another workspace's webhook; 422 for a non-numeric id", async () => {
    const { api } = await tenantApi("p109-iwh-get-404")
    const other = await tenantApi("p109-iwh-get-404-other")
    const foreign = await createWebhook(other.api)
    expectError(await api.get(`${BASE}/1`), 404, "notFound", { defined: true, message: "Inbound webhook not found" })
    expectError(await api.get(`${BASE}/${foreign.id}`), 404, "notFound", { defined: true })
    expectInvalid(await api.get(`${BASE}/abc`), "id")
  })
})

describe("inboundWebhooks.update", () => {
  it("replaces the whole configuration, stores JSON paths in the canonical $ form", async () => {
    const start = Date.now()
    const { api } = await tenantApi("p109-iwh-update")
    const { inboxId } = await createApiChannel(api)
    const tagId = await createTag(api)
    const fieldId = await createCustomField(api, uniqueName("tier"), "shortText")
    const flowId = await createFlow(api, { text: "hello from the webhook flow" })
    const created = await createWebhook(api)
    const sampleJson = { contact: { id: "c-1", "first-name": "Ada" }, items: [{ sku: "A" }] }

    const response = await putConfig(api, created.id, {
      inboxResolution: { mode: "inbox", inboxId },
      identifiers: [
        { kind: "channelUserId", jsonPath: "contact.id" },
        { kind: "email", jsonPath: "  $.contact.email  " },
        { kind: "fullName", jsonPath: "$['contact']['name']" },
      ],
      fieldMappings: [
        { jsonPath: '$.contact["first-name"]', target: "first_name" },
        { jsonPath: "items.0.sku", target: fieldId },
        { jsonPath: "$.gender", target: " gender " },
      ],
      actions: [
        { type: "addTag", tagIds: [tagId] },
        { type: "setCustomField", customFieldId: fieldId, operation: "O01", value: "vip" },
        { type: "startAnotherFlow", flowId },
        { type: "transferConversationToHuman", notifyAdmins: false },
      ],
      sampleJson,
    })
    expect(response.status, response.text).toBe(200)
    expect(Object.keys(response.json).sort()).toEqual(RESOURCE_KEYS)
    expect(response.json).toEqual({
      id: created.id,
      name: created.name,
      active: false,
      createdAt: created.createdAt,
      updatedAt: expect.any(String),
      requestCount: 0,
      endpointUrl: created.endpointUrl,
      inboxResolution: { mode: "inbox", inboxId },
      identifiers: [
        { kind: "channelUserId", jsonPath: "$.contact.id" },
        { kind: "email", jsonPath: "$.contact.email" },
        { kind: "fullName", jsonPath: "$.contact.name" },
      ],
      fieldMappings: [
        { jsonPath: '$.contact["first-name"]', target: "first_name" },
        { jsonPath: "$.items[0].sku", target: fieldId },
        { jsonPath: "$.gender", target: "gender" },
      ],
      actions: [
        { type: "addTag", tagIds: [tagId] },
        { type: "setCustomField", customFieldId: fieldId, operation: "O01", value: "vip" },
        { type: "startAnotherFlow", flowId },
        { type: "transferConversationToHuman", notifyAdmins: false },
      ],
      sampleJson,
    })
    expectIso(response.json.updatedAt, start)
    expect(Date.parse(response.json.updatedAt)).toBeGreaterThanOrEqual(Date.parse(created.updatedAt))

    // GET returns the stored configuration.
    const get = await api.get<Webhook>(`${BASE}/${created.id}`)
    expect(get.json).toEqual(response.json)

    // The body must carry sampleJson (the route doc says it may be omitted).
    expectInvalid(await api.put(`${BASE}/${created.id}`, apiConfig(inboxId)), "sampleJson")
    // A second save replaces everything; a null sampleJson clears it.
    const replaced = await configure(api, created.id, { ...apiConfig(inboxId), sampleJson: null })
    expect(replaced).toMatchObject({
      identifiers: [{ kind: "channelUserId", jsonPath: "$.contact.id" }],
      fieldMappings: [{ jsonPath: "$.contact.first", target: "first_name" }],
      actions: [],
      sampleJson: null,
    })
  })

  it("accepts a payload-resolved inbox rule for a channel that has the identifier kind", async () => {
    const { api } = await tenantApi("p109-iwh-update-payload")
    const created = await createWebhook(api)
    const saved = await configure(api, created.id, {
      inboxResolution: { mode: "payload", channel: "messenger", identifierKind: "pageId", jsonPath: "entry[0].id" },
      identifiers: [{ kind: "channelUserId", jsonPath: "$.sender.id" }],
      fieldMappings: [{ jsonPath: "$.name", target: "full_name" }],
      actions: [],
    })
    expect(saved.inboxResolution).toEqual({
      mode: "payload",
      channel: "messenger",
      identifierKind: "pageId",
      jsonPath: "$.entry[0].id",
    })
  })

  it("refuses an incomplete configuration (422 validation, first missing part)", async () => {
    const { api } = await tenantApi("p109-iwh-update-incomplete")
    const { inboxId } = await createApiChannel(api)
    const { id } = await createWebhook(api)
    expectValidation(
      await putConfig(api, id, { inboxResolution: null, identifiers: [], fieldMappings: [], actions: [] }),
      "validation.inboundWebhook.inboxRequired",
    )
    expectValidation(
      await putConfig(api, id, apiConfig(inboxId, { identifiers: [], fieldMappings: [] })),
      "validation.inboundWebhook.identifierRequired",
    )
    expectValidation(
      await putConfig(api, id, apiConfig(inboxId, { fieldMappings: [] })),
      "validation.inboundWebhook.fieldMappingRequired",
    )
    // Nothing was saved.
    const get = await api.get<Webhook>(`${BASE}/${id}`)
    expect(get.json).toMatchObject({ inboxResolution: null, identifiers: [], fieldMappings: [], actions: [] })
  })

  it("refuses an inbox that is not in the workspace, or an identifier kind the channel lacks", async () => {
    const { api } = await tenantApi("p109-iwh-update-inbox")
    const other = await tenantApi("p109-iwh-update-inbox-other")
    const foreignInbox = (await createApiChannel(other.api)).inboxId
    const { id } = await createWebhook(api)
    expectValidation(await putConfig(api, id, apiConfig("1")), "validation.inboundWebhook.inboxNotFound")
    expectValidation(await putConfig(api, id, apiConfig(foreignInbox)), "validation.inboundWebhook.inboxNotFound")
    expectValidation(
      await putConfig(
        api,
        id,
        apiConfig("1", {
          inboxResolution: { mode: "payload", channel: "webchat", identifierKind: "pageId", jsonPath: "$.page" },
        }),
      ),
      "validation.inboundWebhook.unsupportedInboxIdentifier",
    )
  })

  it("refuses unknown custom-field targets and action references of another workspace", async () => {
    const { api } = await tenantApi("p109-iwh-update-refs")
    const other = await tenantApi("p109-iwh-update-refs-other")
    const { inboxId } = await createApiChannel(api)
    const foreignField = await createCustomField(other.api, uniqueName("f"), "shortText")
    const foreignFlow = await createFlow(other.api, { text: "foreign" })
    const foreignTag = await createTag(other.api)
    const { id } = await createWebhook(api)
    const mapping = (target: string) => apiConfig(inboxId, { fieldMappings: [{ jsonPath: "$.x", target }] })
    // A target that is neither a system field nor a numeric id, an unknown id, a foreign id.
    for (const target of ["nickname", "1", foreignField]) {
      expectValidation(await putConfig(api, id, mapping(target)), "validation.inboundWebhook.unknownFieldTarget")
    }
    for (const action of [
      { type: "startAnotherFlow", flowId: foreignFlow },
      { type: "startAnotherFlow", flowId: "1" },
      { type: "addTag", tagIds: [foreignTag] },
      { type: "removeTag", tagIds: ["1"] },
      { type: "setCustomField", customFieldId: foreignField, operation: "O01", value: "x" },
      { type: "clearCustomField", customFieldId: "1" },
    ]) {
      expectValidation(
        await putConfig(api, id, apiConfig(inboxId, { actions: [action] })),
        "validation.inboundWebhook.actionReferenceNotFound",
      )
    }
  })

  it("validates the request schema (422 invalidRequestData)", async () => {
    const { api } = await tenantApi("p109-iwh-update-schema")
    const { inboxId } = await createApiChannel(api)
    const { id } = await createWebhook(api)
    const put = (body: unknown) => api.put(`${BASE}/${id}`, body)
    const base = { ...apiConfig(inboxId), sampleJson: null }
    const { actions: _a, ...noActions } = base
    expectInvalid(await put(noActions), "actions")
    const { identifiers: _i, ...noIdentifiers } = base
    expectInvalid(await put(noIdentifiers), "identifiers")
    expectInvalid(await put({ ...base, inboxResolution: { mode: "inbox", inboxId: "abc" } }), "inboxResolution.inboxId")
    expectInvalid(await put({ ...base, inboxResolution: { mode: "other" } }), "inboxResolution.mode")
    expectInvalid(
      await put({
        ...base,
        inboxResolution: { mode: "payload", channel: "fax", identifierKind: "pageId", jsonPath: "$.p" },
      }),
      "inboxResolution.channel",
    )
    // Each identity kind once, at most 4 identifiers.
    expectInvalid(
      await put({
        ...base,
        identifiers: [
          { kind: "email", jsonPath: "$.a" },
          { kind: "email", jsonPath: "$.b" },
        ],
      }),
      "identifiers",
    )
    expectInvalid(await put({ ...base, identifiers: [{ kind: "username", jsonPath: "$.a" }] }), "identifiers.0.kind")
    // Unsupported JSON paths: the root alone, a wildcard, an unterminated bracket, empty.
    for (const jsonPath of ["$", "$.items[*]", "$.a[", "  "]) {
      expectInvalid(await put({ ...base, identifiers: [{ kind: "email", jsonPath }] }), "identifiers.0.jsonPath")
    }
    expectInvalid(await put({ ...base, fieldMappings: [{ jsonPath: `$.${"a".repeat(500)}`, target: "email" }] }), "fieldMappings.0.jsonPath")
    expectInvalid(await put({ ...base, fieldMappings: [{ jsonPath: "$.a", target: "  " }] }), "fieldMappings.0.target")
    expectInvalid(
      await put({
        ...base,
        fieldMappings: Array.from({ length: 51 }, (_, index) => ({ jsonPath: `$.f${index}`, target: "email" })),
      }),
      "fieldMappings",
    )
    // Actions: only the six allowed types, each with its own fields, at most 10.
    expectInvalid(await put({ ...base, actions: [{ type: "addSequence", sequenceIds: ["1"] }] }), "actions.0")
    // The branch with the matching type literal reports its own field.
    expectInvalid(await put({ ...base, actions: [{ type: "addTag", tagIds: [] }] }), "actions.0.tagIds")
    expectInvalid(
      await put({ ...base, actions: Array.from({ length: 11 }, () => ({ type: "transferConversationToHuman", notifyAdmins: true })) }),
      "actions",
    )
    // The sample JSON must be storable and at most 64 KiB.
    expectInvalid(await put({ ...base, sampleJson: { text: "a\u0000b" } }), "sampleJson")
    expectInvalid(await put({ ...base, sampleJson: { blob: "x".repeat(64 * 1024) } }), "sampleJson")
  })

  it("404 notFound for an unknown or foreign webhook (before the config is checked)", async () => {
    const { api } = await tenantApi("p109-iwh-update-404")
    const other = await tenantApi("p109-iwh-update-404-other")
    const foreign = await createWebhook(other.api)
    const empty = { inboxResolution: null, identifiers: [], fieldMappings: [], actions: [] }
    expectError(await putConfig(api, "1", empty), 404, "notFound", { defined: true, message: "Inbound webhook not found" })
    expectError(await putConfig(api, foreign.id, empty), 404, "notFound", { defined: true })
  })
})

describe("inboundWebhooks.updateSettings", () => {
  it("renames, activates a complete webhook and deactivates it again", async () => {
    const { api } = await tenantApi("p109-iwh-settings")
    const { inboxId } = await createApiChannel(api)
    const created = await createWebhook(api)
    const patch = (body: unknown) => api.patch<Webhook>(`${BASE}/${created.id}/settings`, body)

    const renamed = await patch({ name: "  Renamed  " })
    expect(renamed.status, renamed.text).toBe(200)
    expect(Object.keys(renamed.json).sort()).toEqual(RESOURCE_KEYS)
    expect(renamed.json).toMatchObject({ id: created.id, name: "Renamed", active: false, endpointUrl: created.endpointUrl })

    // An empty patch changes nothing and returns the webhook.
    const same = await patch({})
    expect(same.status).toBe(200)
    expect(same.json).toEqual(renamed.json)

    // Activation needs a complete configuration.
    expectValidation(await patch({ active: true }), "validation.inboundWebhook.inboxRequired")
    await configure(api, created.id, apiConfig(inboxId))
    const activated = await patch({ active: true })
    expect(activated.status, activated.text).toBe(200)
    expect(activated.json).toMatchObject({ active: true, name: "Renamed", inboxResolution: { mode: "inbox", inboxId } })
    // Activating an active webhook again is accepted.
    expect((await patch({ active: true, name: "Again" })).json).toMatchObject({ active: true, name: "Again" })

    const deactivated = await patch({ active: false })
    expect(deactivated.json.active).toBe(false)
    const list = await api.get<{ data: WebhookSummary[] }>(BASE)
    expect(list.json.data).toEqual([expect.objectContaining({ id: created.id, name: "Again", active: false })])
  })

  it("validates the input and answers 404 for an unknown webhook", async () => {
    const { api } = await tenantApi("p109-iwh-settings-bad")
    const { id } = await createWebhook(api)
    expectInvalid(await api.patch(`${BASE}/${id}/settings`, { name: "" }), "name")
    expectInvalid(await api.patch(`${BASE}/${id}/settings`, { active: "yes" }), "active")
    expectError(await api.patch(`${BASE}/1/settings`, { name: "x" }), 404, "notFound", {
      defined: true,
      message: "Inbound webhook not found",
    })
    expectError(await api.patch(`${BASE}/1/settings`, {}), 404, "notFound", { defined: true })
  })
})

describe("inboundWebhooks.regenerateToken", () => {
  it(
    "returns a new endpoint URL; the old URL stops working, the new one works",
    async () => {
      const { api } = await tenantApi("p109-iwh-regen")
      const { inboxId } = await createApiChannel(api)
      const created = await createWebhook(api)
      await configure(api, created.id, apiConfig(inboxId))
      await setActive(api, created.id, true)
      const oldUrl = created.endpointUrl as string
      // The old URL works before the regeneration.
      expect((await deliver(oldUrl, { contact: { id: "before" } })).status).toBe(202)

      const response = await api.post<{ endpointUrl: string }>(`${BASE}/${created.id}/regenerate-token`)
      expect(response.status, response.text).toBe(200)
      expect(response.json).toEqual({ endpointUrl: expect.stringMatching(ENDPOINT_URL) })
      const newUrl = response.json.endpointUrl
      expect(newUrl).not.toBe(oldUrl)
      expect((await api.get<Webhook>(`${BASE}/${created.id}`)).json.endpointUrl).toBe(newUrl)

      // Within 10 s (token cache TTL) the old URL is unknown.
      await eventually(
        async () => {
          const old = await deliver(oldUrl, { contact: { id: "after" } })
          expect(old.status).toBe(404)
          expect(old.json).toEqual({ accepted: false, code: "NOT_FOUND", message: "Webhook not found" })
        },
        { timeoutMs: 20_000, intervalMs: 1_000, description: "old URL refused" },
      )
      const fresh = await deliver(newUrl, { contact: { id: "after" } })
      expect(fresh.status, fresh.text).toBe(202)
    },
    60_000,
  )

  it("404 notFound for an unknown webhook", async () => {
    const { api } = await tenantApi("p109-iwh-regen-404")
    expectError(await api.post(`${BASE}/1/regenerate-token`), 404, "notFound", {
      defined: true,
      message: "Inbound webhook not found",
    })
  })
})

describe("inboundWebhooks.delete / deleteMany", () => {
  it("deletes one webhook (204); then get/delete 404 and its URL is unknown", async () => {
    const { api } = await tenantApi("p109-iwh-delete")
    const { inboxId } = await createApiChannel(api)
    const created = await createWebhook(api)
    await configure(api, created.id, apiConfig(inboxId))
    await setActive(api, created.id, true)
    const response = await api.delete(`${BASE}/${created.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectError(await api.get(`${BASE}/${created.id}`), 404, "notFound")
    expectError(await api.delete(`${BASE}/${created.id}`), 404, "notFound", {
      defined: true,
      message: "Inbound webhook not found",
    })
    await eventually(
      async () => expect((await deliver(created.endpointUrl as string, { contact: { id: "x" } })).status).toBe(404),
      { timeoutMs: 20_000, intervalMs: 1_000, description: "deleted URL refused" },
    )
  })

  it("deletes several (204); unknown ids and other workspaces' ids are ignored", async () => {
    const { api } = await tenantApi("p109-iwh-delete-many")
    const other = await tenantApi("p109-iwh-delete-many-other")
    const a = await createWebhook(api)
    const b = await createWebhook(api)
    const keep = await createWebhook(api)
    const foreign = await createWebhook(other.api)
    const response = await api.post(`${BASE}/bulk-delete`, { ids: [a.id, b.id, foreign.id, "1"] })
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    const list = await api.get<{ data: WebhookSummary[] }>(BASE)
    expect(list.json.data.map((item) => item.id)).toEqual([keep.id])
    expect((await other.api.get(`${BASE}/${foreign.id}`)).status).toBe(200)
    // Nothing to delete is still 204.
    expect((await api.post(`${BASE}/bulk-delete`, { ids: [] })).status).toBe(204)
    expect((await api.post(`${BASE}/bulk-delete`, { ids: [a.id] })).status).toBe(204)
  })

  it("validates the ids", async () => {
    const { api } = await tenantApi("p109-iwh-delete-many-bad")
    expectInvalid(await api.post(`${BASE}/bulk-delete`, {}), "ids")
    expectInvalid(await api.post(`${BASE}/bulk-delete`, { ids: ["abc"] }), "ids.0")
    expectInvalid(await api.delete(`${BASE}/abc`), "id")
  })
})

describe("a read-only token", () => {
  it("lists and reads but cannot preview (403 FORBIDDEN)", async () => {
    const { tenant, api } = await tenantApi("p109-iwh-readonly")
    const created = await createWebhook(api)
    const readOnly: Api = apiClient(await extraToken(tenant, { permission: "read_only" }))
    const list = await readOnly.get<{ data: WebhookSummary[] }>(BASE)
    expect(list.status).toBe(200)
    expect(list.json.data.map((item) => item.id)).toEqual([created.id])
    expect((await readOnly.get(`${BASE}/${created.id}/requests`)).status).toBe(200)
    expectError(
      await readOnly.post(`${BASE}/preview-mapping`, { payload: {}, inboxResolution: null, identifiers: [], fieldMappings: [] }),
      403,
      "FORBIDDEN",
      { defined: true, message: "Read-only token cannot perform this operation" },
    )
  })
})
