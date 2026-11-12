// messengerTemplates.list / get / create / clone / delete / sync (scope
// `broadcasts`). Meta calls (graph.facebook.com, page token):
//   sync    `GET /v23.0/<pageId>/message_templates?fields=name,status,language,category,parameter_format,components`
//           (a full sync also deletes local templates Meta no longer lists);
//   create  `POST /v23.0/<pageId>/message_templates` {name, language,
//           category UTILITY, components}, then a partial sync of that name;
//   clone   the same POST on each target page; an APPROVED answer is synced
//           there, any other status is reported as failed.
// Page ids are unique per test, so every Graph path is test-owned. Local
// templates for list/get/delete come from the sync route or the fixture
// extension (a sync row).

import { describe, expect, it } from "vitest"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import { unordered } from "../../src/normalize.ts"
import {
  type Api,
  connectMessenger,
  expectInvalid,
  expectNotFound,
  ext,
  GRAPH,
  graphError,
  graphPath,
  keysOf,
  type MessengerPage,
  newTenant,
  parseBody,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueDigits,
  upstreamCalls,
} from "./helpers.ts"

type Template = {
  id: string
  name: string
  language: string
  category: string
  status: string
  components: unknown
  parameterFormat: string
  integrationMessengerId: string
  integrationMessenger?: { id: string; name: string; inboxId: string }
  parameters?: Array<Record<string, unknown>>
}

const TEMPLATE_KEYS = [
  "category",
  "components",
  "id",
  "integrationMessenger",
  "integrationMessengerId",
  "language",
  "name",
  "parameterFormat",
  "status",
]

const COMPONENTS = [{ type: "BODY", text: "Hi {{1}}, your booking is confirmed" }]

const metaTemplate = (id: string, name: string, status = "APPROVED", components: unknown[] = COMPONENTS) => ({
  id,
  name,
  status,
  language: "en_US",
  category: "UTILITY",
  components,
})

const templatesPath = (page: MessengerPage) => new RegExp(graphPath(`${page.pageId}/message_templates`))

const answerList = (page: MessengerPage, data: unknown[]) =>
  fake.script({
    match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${page.pageId}/message_templates`) },
    respond: { status: 200, json: { data } },
  })

const list = async (api: Api, query: Record<string, string> = {}): Promise<Template[]> => {
  const response = await api.get<Template[]>("/v1/messenger/templates", { query })
  expect(response.status, response.text).toBe(200)
  return response.json
}

const uniqueTemplateName = () => `booking_${uniqueDigits().slice(-8)}`

const seedTemplate = async (page: MessengerPage, overrides: { name?: string; status?: "APPROVED" | "PENDING" | "REJECTED" } = {}) =>
  (
    await (await ext()).createMessengerTemplate({
      integrationMessengerId: page.integrationId,
      name: overrides.name ?? uniqueTemplateName(),
      language: "en_US",
      status: overrides.status ?? "APPROVED",
      category: "UTILITY",
      components: COMPONENTS,
    })
  ).templateId

describe("messengerTemplates.sync / list / get", () => {
  it("sync pulls the page's templates (204); list returns them newest id first with their page", async () => {
    const { tenant, api } = await newTenant("p111-tpl-sync")
    const page = await connectMessenger(tenant)
    const approved = uniqueTemplateName()
    const pending = uniqueTemplateName()
    const approvedMetaId = uniqueDigits()
    await answerList(page, [metaTemplate(approvedMetaId, approved), metaTemplate(uniqueDigits(), pending, "PENDING")])
    const since = await fake.cursor()
    const response = await api.post(`/v1/messenger-channels/${page.integrationId}/templates/sync`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    const calls = await upstreamCalls({ host: GRAPH, since, path: templatesPath(page) })
    expect(calls).toHaveLength(1)
    expect(calls[0]?.query).toEqual({ fields: "name,status,language,category,parameter_format,components" })
    expect(calls[0]?.headers.authorization).toBe(`Bearer ${page.accessToken}`)

    const templates = await list(api)
    expect(templates).toHaveLength(2)
    for (const template of templates) {
      expect(keysOf(template)).toEqual(TEMPLATE_KEYS)
      expect(template.integrationMessenger).toEqual({ id: page.integrationId, name: page.name, inboxId: page.inboxId })
    }
    expect(unordered(templates, (t) => t.name)).toEqual(
      unordered(
        [
          {
            id: expect.stringMatching(SNOWFLAKE),
            name: approved,
            language: "en_US",
            category: "UTILITY",
            status: "APPROVED",
            parameterFormat: "POSITIONAL",
            components: COMPONENTS,
            integrationMessengerId: page.integrationId,
            integrationMessenger: { id: page.integrationId, name: page.name, inboxId: page.inboxId },
          },
          {
            id: expect.stringMatching(SNOWFLAKE),
            name: pending,
            language: "en_US",
            category: "UTILITY",
            status: "PENDING",
            parameterFormat: "POSITIONAL",
            components: COMPONENTS,
            integrationMessengerId: page.integrationId,
            integrationMessenger: { id: page.integrationId, name: page.name, inboxId: page.inboxId },
          },
        ],
        (t) => t.name,
      ),
    )
    const ids = templates.map((t) => BigInt(t.id))
    expect([...ids].sort((a, b) => (a < b ? 1 : -1))).toEqual(ids)

    // A later full sync drops templates Meta no longer lists and updates the rest.
    await answerList(page, [metaTemplate(approvedMetaId, approved, "REJECTED")])
    expect((await api.post(`/v1/messenger-channels/${page.integrationId}/templates/sync`)).status).toBe(204)
    const after = await list(api)
    expect(after.map((t) => [t.id, t.name, t.status])).toEqual([
      [templates.find((t) => t.name === approved)?.id, approved, "REJECTED"],
    ])
  })

  it("list filters by integrationMessengerId, inboxId and status; another workspace sees none", async () => {
    const { tenant, api } = await newTenant("p111-tpl-filter")
    const pageA = await connectMessenger(tenant)
    const pageB = await connectMessenger(tenant)
    const a = await seedTemplate(pageA)
    const b = await seedTemplate(pageB, { status: "PENDING" })
    expect((await list(api, { integrationMessengerId: pageA.integrationId })).map((t) => t.id)).toEqual([a])
    expect((await list(api, { inboxId: pageB.inboxId })).map((t) => t.id)).toEqual([b])
    expect((await list(api, { status: "PENDING" })).map((t) => t.id)).toEqual([b])
    expect((await list(api, { inboxId: UNKNOWN_ID }))).toEqual([])
    expect((await list(api)).map((t) => t.id)).toEqual([b, a])
    expectInvalid(await api.get("/v1/messenger/templates", { query: { status: "DRAFT" } }), "status")
    const { api: otherApi } = await newTenant("p111-tpl-filter-o")
    expect(await list(otherApi)).toEqual([])
    expect(await list(otherApi, { integrationMessengerId: pageA.integrationId })).toEqual([])
  })

  it("get returns the template with the keys to fill; unknown or foreign id 404", async () => {
    const { tenant, api } = await newTenant("p111-tpl-get")
    const page = await connectMessenger(tenant)
    const id = await seedTemplate(page)
    const response = await api.get<Template>(`/v1/messenger/templates/${id}`)
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual([...TEMPLATE_KEYS, "parameters"].filter((k) => k !== "integrationMessenger").sort())
    expect(response.json).toMatchObject({ id, status: "APPROVED", components: COMPONENTS, integrationMessengerId: page.integrationId })
    expect(response.json.parameters).toEqual([
      expect.objectContaining({ key: "body.1", component: "body", kind: "text", required: true }),
    ])
    expectNotFound(await api.get(`/v1/messenger/templates/${UNKNOWN_ID}`), "Template not found")
    const { api: otherApi } = await newTenant("p111-tpl-get-o")
    expectNotFound(await otherApi.get(`/v1/messenger/templates/${id}`), "Template not found")
    expectInvalid(await api.get("/v1/messenger/templates/abc"), "id")
  })

  it("sync: a Meta error is 400; an unknown or foreign page is 404", async () => {
    const { tenant, api } = await newTenant("p111-tpl-sync-err")
    const page = await connectMessenger(tenant)
    await fake.script({
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${page.pageId}/message_templates`) },
      respond: { status: 400, json: graphError("(#100) Unsupported get request", 100) },
    })
    const failed = await api.post(`/v1/messenger-channels/${page.integrationId}/templates/sync`)
    expect(failed.status, failed.text).toBe(400)
    expect((failed.json as { message: string }).message).toContain("Failed to fetch Messenger templates from Facebook API")
    expectNotFound(await api.post(`/v1/messenger-channels/${UNKNOWN_ID}/templates/sync`), "Messenger channel not found")
    const { api: otherApi } = await newTenant("p111-tpl-sync-err-o")
    expectNotFound(await otherApi.post(`/v1/messenger-channels/${page.integrationId}/templates/sync`), "Messenger channel not found")
  })
})

describe("messengerTemplates.create", () => {
  it("submits a utility template to Meta, mirrors it locally and returns Meta's and the local id", async () => {
    const { tenant, api } = await newTenant("p111-tpl-create")
    const page = await connectMessenger(tenant)
    const name = uniqueTemplateName()
    const metaId = uniqueDigits()
    await fake.script(
      {
        match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${page.pageId}/message_templates`) },
        respond: { status: 200, json: { id: metaId, status: "PENDING", category: "UTILITY" } },
      },
      {
        match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${page.pageId}/message_templates`) },
        respond: {
          status: 200,
          json: {
            data: [
              metaTemplate(metaId, name, "PENDING", [
                { type: "BODY", text: "Hello {{1}}", example: { body_text: [["Ada"]] } },
                { type: "BUTTONS", buttons: [{ type: "POSTBACK", text: "Yes", payload: "{{1}}" }] },
              ]),
            ],
          },
        },
      },
    )
    const since = await fake.cursor()
    const response = await api.post<{
      id: string
      templateId: string | null
      status: string
      rejectionReason: string | null
      specificRejectionReason: string | null
    }>(`/v1/messenger-channels/${page.integrationId}/templates`, {
      name,
      language: "en_US",
      headerType: "none",
      body: "Hello {{1}}",
      bodyVariables: [{ key: "{{1}}", example: "Ada" }],
      buttons: [{ type: "POSTBACK", title: "Yes" }],
    })
    expect(response.status, response.text).toBe(201)
    expect(response.json).toEqual({
      id: metaId,
      templateId: expect.stringMatching(SNOWFLAKE),
      status: "PENDING",
      rejectionReason: null,
      specificRejectionReason: null,
    })
    const calls = await upstreamCalls({ host: GRAPH, since, path: templatesPath(page) })
    expect(calls.map((call) => call.method)).toEqual(["POST", "GET"])
    const body = parseBody<{ name: string; language: string; category: string; components: unknown[] }>(
      calls[0] as UpstreamRecord,
    )
    expect(body).toMatchObject({ name, language: "en_US", category: "UTILITY" })
    expect(body.components).toEqual([
      { type: "BODY", text: "Hello {{1}}", example: { body_text: [["Ada"]] } },
      { type: "BUTTONS", buttons: [{ type: "POSTBACK", text: "Yes", payload: "{{1}}" }] },
    ])
    expect(calls[1]?.query).toMatchObject({ name })

    const local = await api.get<Template>(`/v1/messenger/templates/${response.json.templateId}`)
    expect(local.status, local.text).toBe(200)
    expect(local.json).toMatchObject({ name, status: "PENDING", integrationMessengerId: page.integrationId })
  })

  it("validates the request (422) and is 404 for an unknown page; Meta's rejection is 400", async () => {
    const { tenant, api } = await newTenant("p111-tpl-create-err")
    const page = await connectMessenger(tenant)
    const path = `/v1/messenger-channels/${page.integrationId}/templates`
    const base = { name: uniqueTemplateName(), language: "en_US", headerType: "none", body: "Hello" }
    const since = await fake.cursor()
    expectInvalid(await api.post(path, { ...base, name: "Has Caps" }), "name")
    expectInvalid(await api.post(path, { ...base, body: "Hi {{1}}" }), "bodyVariables")
    expectInvalid(await api.post(path, { ...base, headerType: "text" }), "headerText")
    expectInvalid(await api.post(path, { ...base, headerType: "text_and_image", headerText: "H" }), "headerImageUrl")
    expectInvalid(
      await api.post(path, { ...base, buttons: [{ type: "POSTBACK", title: "x".repeat(21) }] }),
      "buttons.0.title",
    )
    expect(await upstreamCalls({ host: GRAPH, since, path: templatesPath(page) })).toEqual([])
    expectNotFound(await api.post(`/v1/messenger-channels/${UNKNOWN_ID}/templates`, base), "Messenger channel not found")

    await fake.script({
      match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${page.pageId}/message_templates`) },
      respond: { status: 400, json: graphError("(#100) Invalid parameter", 100) },
    })
    const rejected = await api.post(path, base)
    expect(rejected.status, rejected.text).toBe(400)
  })
})

describe("messengerTemplates.clone", () => {
  const twoPages = async (label: string): Promise<{ tenant: Tenant; api: Api; source: MessengerPage; target: MessengerPage; pending: MessengerPage }> => {
    const { tenant, api } = await newTenant(label)
    return {
      tenant,
      api,
      source: await connectMessenger(tenant),
      target: await connectMessenger(tenant),
      pending: await connectMessenger(tenant),
    }
  }

  it("creates the template on each target page; APPROVED is synced there, PENDING is reported failed", async () => {
    const { api, source, target, pending } = await twoPages("p111-tpl-clone")
    const name = uniqueTemplateName()
    const id = await seedTemplate(source, { name })
    const approvedId = uniqueDigits()
    await fake.script(
      {
        match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${target.pageId}/message_templates`) },
        respond: { status: 200, json: { id: approvedId, status: "APPROVED", category: "UTILITY" } },
      },
      {
        match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${target.pageId}/message_templates`) },
        respond: { status: 200, json: { data: [metaTemplate(approvedId, name)] } },
      },
      {
        match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${pending.pageId}/message_templates`) },
        respond: { status: 200, json: { id: uniqueDigits(), status: "PENDING", category: "UTILITY" } },
      },
    )
    const since = await fake.cursor()
    const response = await api.post<{ succeeded: Array<{ channel: string }>; failed: Array<{ channel: string; error: string }> }>(
      `/v1/messenger/templates/${id}/clone`,
      { targetIntegrationMessengerIds: [target.integrationId, pending.integrationId, source.integrationId] },
    )
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      succeeded: [{ channel: target.name }],
      failed: [{ channel: pending.name, error: "Template returned status: PENDING" }],
    })
    const posts = await upstreamCalls({ host: GRAPH, since, path: /message_templates$/, method: "POST" })
    expect(posts.map((call) => call.headers.authorization).sort()).toEqual(
      [`Bearer ${target.accessToken}`, `Bearer ${pending.accessToken}`].sort(),
    )
    expect(parseBody(posts[0] as UpstreamRecord)).toMatchObject({
      name,
      language: "en_US",
      category: "UTILITY",
      parameter_format: "POSITIONAL",
      components: COMPONENTS,
    })
    expect((await list(api, { integrationMessengerId: target.integrationId })).map((t) => t.name)).toEqual([name])
    expect(await list(api, { integrationMessengerId: pending.integrationId })).toEqual([])
  })

  it("no target other than the template's own page -> 404; unknown template 404; body validated", async () => {
    const { tenant, api, source } = await twoPages("p111-tpl-clone-err")
    const id = await seedTemplate(source)
    expectNotFound(
      await api.post(`/v1/messenger/templates/${id}/clone`, { targetIntegrationMessengerIds: [source.integrationId] }),
      "No target channels found in this workspace",
    )
    const { tenant: other } = await newTenant("p111-tpl-clone-err-o")
    const foreign = await connectMessenger(other)
    expectNotFound(
      await api.post(`/v1/messenger/templates/${id}/clone`, { targetIntegrationMessengerIds: [foreign.integrationId] }),
      "No target channels found in this workspace",
    )
    expectNotFound(
      await api.post(`/v1/messenger/templates/${UNKNOWN_ID}/clone`, { targetIntegrationMessengerIds: [source.integrationId] }),
      "Template not found",
    )
    expectInvalid(await api.post(`/v1/messenger/templates/${id}/clone`, { targetIntegrationMessengerIds: [] }), "targetIntegrationMessengerIds")
    expect(tenant.workspaceId).toMatch(SNOWFLAKE)
  })
})

describe("messengerTemplates.delete", () => {
  it("removes the local copy only (204); a second delete and a foreign id are 404", async () => {
    const { tenant, api } = await newTenant("p111-tpl-delete")
    const page = await connectMessenger(tenant)
    const id = await seedTemplate(page)
    const since = await fake.cursor()
    const response = await api.delete(`/v1/messenger/templates/${id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expect(await upstreamCalls({ host: GRAPH, since, path: templatesPath(page) })).toEqual([])
    expect(await list(api)).toEqual([])
    expectNotFound(await api.delete(`/v1/messenger/templates/${id}`), "Template not found")
    const kept = await seedTemplate(page)
    const { api: otherApi } = await newTenant("p111-tpl-delete-o")
    expectNotFound(await otherApi.delete(`/v1/messenger/templates/${kept}`), "Template not found")
    expect((await list(api)).map((t) => t.id)).toEqual([kept])
  })
})
