// miniApps.pushWhatsappDraft / publishWhatsapp / publishWhatsappDraft /
// getWhatsappPreview / deleteWhatsappDraft, and miniApps.delete with
// `deleteWhatsappFlows`. Meta's WhatsApp Flows API is the fake upstream
// `graph.facebook.com`; every path holds a per-test WABA id or Flow id. The
// same `GET /<flowId>` serves the Flow status and the preview, so each script
// answers once (`times`) and is consumed by the test in order (the newest
// script answers first).

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import {
  type Api,
  ext,
  expectError,
  expectInvalid,
  expectNotFound,
  expectValidation,
  GRAPH,
  graphPath,
  ISO,
  keysOf,
  newTenant,
  parseBody,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueDigits,
  uniqueName,
  upstreamCalls,
} from "./helpers.ts"

type Publish = { status: string; published: boolean; whatsappFlowId: string | null; validationErrors: unknown[] }
type MiniApp = {
  id: string
  name: string
  flowJson: Record<string, unknown>
  publications: Array<{ integrationWhatsappId: string; status: string; whatsappFlowId: string | null; publishedAt: string | null }>
}

type Setup = { tenant: Tenant; api: Api; app: MiniApp; integrationId: string; wabaId: string; accessToken: string }

const setup = async (label: string): Promise<Setup> => {
  const { tenant, api } = await newTenant(label)
  const channel = await (await ext()).createWhatsappChannel({
    workspaceId: tenant.workspaceId,
    phoneNumberId: uniqueDigits(),
    name: uniqueName("WA number"),
  })
  const created = await api.post<MiniApp>("/v1/mini-apps", { name: uniqueName("Signup") })
  expect(created.status, created.text).toBe(201)
  return {
    tenant,
    api,
    app: created.json,
    integrationId: channel.integrationId,
    wabaId: channel.wabaId,
    accessToken: channel.accessToken,
  }
}

const flow = (flowId: string, name: string, status: string) => ({
  id: flowId,
  name,
  status,
  categories: ["OTHER"],
  validation_errors: [],
})

/** Meta creates the Flow (`POST /<wabaId>/flows`) and reports it as `status` once. */
const scriptCreate = async (s: Setup, flowId: string, status = "DRAFT") => {
  await fake.script(
    {
      match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${s.wabaId}/flows`) },
      respond: { status: 200, json: { id: flowId, success: true, validation_errors: [] } },
      times: 1,
    },
    {
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(flowId) },
      respond: { status: 200, json: flow(flowId, s.app.name, status) },
      times: 1,
    },
  )
}

/** Meta publishes a DRAFT Flow: GET (DRAFT) -> POST /publish -> GET (PUBLISHED). */
const scriptPublishDraft = async (s: Setup, flowId: string) => {
  // Older script first: it answers the second GET once the newer one is used up.
  await fake.script({
    match: { host: GRAPH, method: "GET", pathRegex: graphPath(flowId) },
    respond: { status: 200, json: flow(flowId, s.app.name, "PUBLISHED") },
    times: 1,
  })
  await fake.script(
    {
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(flowId) },
      respond: { status: 200, json: flow(flowId, s.app.name, "DRAFT") },
      times: 1,
    },
    {
      match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${flowId}/publish`) },
      respond: { status: 200, json: { success: true } },
      times: 1,
    },
  )
}

const pushDraft = (s: Setup) =>
  s.api.post<Publish>(`/v1/mini-apps/${s.app.id}/whatsapp-draft`, { integrationWhatsappId: s.integrationId })

const getApp = async (s: Setup): Promise<MiniApp> => {
  const response = await s.api.get<MiniApp>(`/v1/mini-apps/${s.app.id}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("miniApps.pushWhatsappDraft", () => {
  it("creates the Flow on Meta as a draft and records the publication", async () => {
    const s = await setup("p112-maw-push")
    const flowId = uniqueDigits()
    await scriptCreate(s, flowId)
    const since = await fake.cursor()
    const response = await pushDraft(s)
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(["published", "status", "validationErrors", "whatsappFlowId"])
    expect(response.json).toEqual({
      status: "DRAFT",
      published: false,
      whatsappFlowId: expect.stringMatching(SNOWFLAKE),
      validationErrors: [],
    })

    const calls = await upstreamCalls({ host: GRAPH, since, bearer: s.accessToken })
    expect(calls.map((call) => `${call.method} ${call.path.replace(/^\/v[0-9.]+/, "")}`)).toEqual([
      `POST /${s.wabaId}/flows`,
      `GET /${flowId}`,
    ])
    expect(parseBody(calls[0] as (typeof calls)[number])).toEqual({
      name: s.app.name,
      categories: ["OTHER"],
      flow_json: JSON.stringify(s.app.flowJson),
    })
    expect(calls[1]?.query.fields).toBe("id,name,status,categories,validation_errors")

    expect((await getApp(s)).publications).toEqual([
      {
        integrationWhatsappId: s.integrationId,
        status: "DRAFT",
        whatsappFlowId: response.json.whatsappFlowId,
        publishedAt: null,
      },
    ])
  })

  it("404 for an unknown WhatsApp number or Mini App; 422 without a number", async () => {
    const s = await setup("p112-maw-push-404")
    const since = await fake.cursor()
    expectNotFound(
      await s.api.post(`/v1/mini-apps/${s.app.id}/whatsapp-draft`, { integrationWhatsappId: UNKNOWN_ID }),
      "WhatsApp number not found",
    )
    expectNotFound(
      await s.api.post(`/v1/mini-apps/${UNKNOWN_ID}/whatsapp-draft`, { integrationWhatsappId: s.integrationId }),
      "Mini App not found",
    )
    expectInvalid(await s.api.post(`/v1/mini-apps/${s.app.id}/whatsapp-draft`, {}), "integrationWhatsappId")
    expect(await upstreamCalls({ host: GRAPH, since, bearer: s.accessToken })).toHaveLength(0)
  })

  it("refuses to publish an invalid Mini App before calling Meta", async () => {
    const s = await setup("p112-maw-invalid")
    const broken = await s.api.patch(`/v1/mini-apps/${s.app.id}`, {
      flowJson: {
        version: "7.2",
        screens: [{ id: "ONLY", layout: { type: "SingleColumnLayout", children: [{ type: "TextBody", text: "x" }] } }],
      },
    })
    expect(broken.status, broken.text).toBe(200)
    const since = await fake.cursor()
    const response = await s.api.post(`/v1/mini-apps/${s.app.id}/publish-whatsapp`, { integrationWhatsappId: s.integrationId })
    const body = expectError(response, 422, "validation", { defined: true })
    expect(body.message).toMatch(/^Fix the Mini App errors before publishing \(errors=\d+\)$/)
    expect(await upstreamCalls({ host: GRAPH, since, bearer: s.accessToken })).toHaveLength(0)
  })
})

describe("miniApps.publishWhatsapp", () => {
  it("creates and publishes the Flow; the publication is PUBLISHED with a time", async () => {
    const s = await setup("p112-maw-publish")
    const flowId = uniqueDigits()
    await scriptPublishDraft(s, flowId)
    await fake.script({
      match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${s.wabaId}/flows`) },
      respond: { status: 200, json: { id: flowId, success: true, validation_errors: [] } },
      times: 1,
    })
    const since = await fake.cursor()
    const response = await s.api.post<Publish>(`/v1/mini-apps/${s.app.id}/publish-whatsapp`, {
      integrationWhatsappId: s.integrationId,
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      status: "PUBLISHED",
      published: true,
      whatsappFlowId: expect.stringMatching(SNOWFLAKE),
      validationErrors: [],
    })
    const calls = await upstreamCalls({ host: GRAPH, since, bearer: s.accessToken })
    expect(calls.map((call) => `${call.method} ${call.path.replace(/^\/v[0-9.]+/, "")}`)).toEqual([
      `POST /${s.wabaId}/flows`,
      `GET /${flowId}`,
      `POST /${flowId}/publish`,
      `GET /${flowId}`,
    ])
    const [publication] = (await getApp(s)).publications
    expect(publication).toEqual({
      integrationWhatsappId: s.integrationId,
      status: "PUBLISHED",
      whatsappFlowId: response.json.whatsappFlowId,
      publishedAt: expect.stringMatching(ISO),
    })
  })
})

describe("miniApps.publishWhatsappDraft / getWhatsappPreview / deleteWhatsappDraft", () => {
  it("previews a draft, publishes it, and refuses to delete it once live", async () => {
    const s = await setup("p112-maw-draft")
    const flowId = uniqueDigits()
    await scriptCreate(s, flowId)
    const pushed = await pushDraft(s)
    expect(pushed.status, pushed.text).toBe(200)

    await fake.script({
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(flowId) },
      respond: {
        status: 200,
        json: { id: flowId, preview: { preview_url: `https://business.facebook.com/wa/manage/flows/${flowId}/preview/`, expires_at: "2030-01-01T00:00:00+0000" } },
      },
      times: 1,
    })
    let since = await fake.cursor()
    const preview = await s.api.get<{ previewUrl: string | null; expiresAt: string | null }>(
      `/v1/mini-apps/${s.app.id}/whatsapp-preview`,
      { query: { integrationWhatsappId: s.integrationId } },
    )
    expect(preview.status, preview.text).toBe(200)
    expect(preview.json).toEqual({
      previewUrl: `https://business.facebook.com/wa/manage/flows/${flowId}/preview/`,
      expiresAt: "2030-01-01T00:00:00+0000",
    })
    const previewCalls = await upstreamCalls({ host: GRAPH, since, bearer: s.accessToken })
    expect(previewCalls).toHaveLength(1)
    expect(previewCalls[0]?.query.fields).toBe("preview.invalidate(false)")

    await scriptPublishDraft(s, flowId)
    since = await fake.cursor()
    const published = await s.api.post<Publish>(`/v1/mini-apps/${s.app.id}/whatsapp-draft/publish`, {
      integrationWhatsappId: s.integrationId,
    })
    expect(published.status, published.text).toBe(200)
    expect(published.json).toEqual({
      status: "PUBLISHED",
      published: true,
      whatsappFlowId: pushed.json.whatsappFlowId,
      validationErrors: [],
    })
    const publishCalls = await upstreamCalls({ host: GRAPH, since, bearer: s.accessToken })
    expect(publishCalls.map((call) => `${call.method} ${call.path.replace(/^\/v[0-9.]+/, "")}`)).toEqual([
      `GET /${flowId}`,
      `POST /${flowId}/publish`,
      `GET /${flowId}`,
    ])

    await fake.script({
      match: { host: GRAPH, method: "GET", pathRegex: graphPath(flowId) },
      respond: { status: 200, json: flow(flowId, s.app.name, "PUBLISHED") },
      times: 1,
    })
    since = await fake.cursor()
    expectValidation(
      await s.api.delete(`/v1/mini-apps/${s.app.id}/whatsapp-draft/${s.integrationId}`),
      "The Flow is live on Meta, so it can't be deleted. Only a draft can be deleted.",
    )
    const deleteCalls = await upstreamCalls({ host: GRAPH, since, bearer: s.accessToken })
    expect(deleteCalls.map((call) => call.method)).toEqual(["GET"])
    expect((await getApp(s)).publications).toMatchObject([{ status: "PUBLISHED" }])
  })

  it("deletes a draft Flow on Meta and drops the publication", async () => {
    const s = await setup("p112-maw-delete-draft")
    const flowId = uniqueDigits()
    await scriptCreate(s, flowId)
    expect((await pushDraft(s)).status).toBe(200)
    await fake.script(
      {
        match: { host: GRAPH, method: "GET", pathRegex: graphPath(flowId) },
        respond: { status: 200, json: flow(flowId, s.app.name, "DRAFT") },
        times: 1,
      },
      {
        match: { host: GRAPH, method: "DELETE", pathRegex: graphPath(flowId) },
        respond: { status: 200, json: { success: true } },
        times: 1,
      },
    )
    const since = await fake.cursor()
    const response = await s.api.delete<{ outcome: string }>(`/v1/mini-apps/${s.app.id}/whatsapp-draft/${s.integrationId}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ outcome: "deleted" })
    const calls = await upstreamCalls({ host: GRAPH, since, bearer: s.accessToken })
    expect(calls.map((call) => `${call.method} ${call.path.replace(/^\/v[0-9.]+/, "")}`)).toEqual([
      `GET /${flowId}`,
      `DELETE /${flowId}`,
    ])
    expect((await getApp(s)).publications).toEqual([])
  })

  it("404 when the Mini App is not on the number", async () => {
    const s = await setup("p112-maw-not-on")
    const since = await fake.cursor()
    const message = "The Mini App is not on this WhatsApp number."
    expectNotFound(
      await s.api.get(`/v1/mini-apps/${s.app.id}/whatsapp-preview`, { query: { integrationWhatsappId: s.integrationId } }),
      message,
    )
    expectNotFound(
      await s.api.post(`/v1/mini-apps/${s.app.id}/whatsapp-draft/publish`, { integrationWhatsappId: s.integrationId }),
      message,
    )
    expectNotFound(await s.api.delete(`/v1/mini-apps/${s.app.id}/whatsapp-draft/${s.integrationId}`), message)
    expectNotFound(
      await s.api.get(`/v1/mini-apps/${UNKNOWN_ID}/whatsapp-preview`, { query: { integrationWhatsappId: s.integrationId } }),
      "Mini App not found",
    )
    expectInvalid(await s.api.get(`/v1/mini-apps/${s.app.id}/whatsapp-preview`), "integrationWhatsappId")
    expect(await upstreamCalls({ host: GRAPH, since, bearer: s.accessToken })).toHaveLength(0)
  })
})

describe("miniApps.delete with deleteWhatsappFlows", () => {
  it("removes the draft Flow on Meta and reports each Flow's outcome", async () => {
    const s = await setup("p112-maw-delete-app")
    const flowId = uniqueDigits()
    await scriptCreate(s, flowId)
    expect((await pushDraft(s)).status).toBe(200)
    await fake.script(
      {
        match: { host: GRAPH, method: "GET", pathRegex: graphPath(flowId) },
        respond: { status: 200, json: flow(flowId, s.app.name, "DRAFT") },
        times: 1,
      },
      {
        match: { host: GRAPH, method: "DELETE", pathRegex: graphPath(flowId) },
        respond: { status: 200, json: { success: true } },
        times: 1,
      },
    )
    const response = await s.api.delete<{ deletedCount: number; whatsappFlows: unknown[] }>(`/v1/mini-apps/${s.app.id}`, {
      json: { deleteWhatsappFlows: true },
    })
    expect(response.status, response.text).toBe(200)
    // Each entry also carries `miniAppId` and `flowId` (Meta's Flow id), which the route doc omits.
    expect(response.json).toEqual({
      deletedCount: 1,
      whatsappFlows: [{ integrationWhatsappId: s.integrationId, miniAppId: s.app.id, flowId, outcome: "deleted" }],
    })
    expectNotFound(await s.api.get(`/v1/mini-apps/${s.app.id}`), "Mini App not found")
  })

  it("without the flag the Flows stay on Meta (no call)", async () => {
    const s = await setup("p112-maw-delete-keep")
    const flowId = uniqueDigits()
    await scriptCreate(s, flowId)
    expect((await pushDraft(s)).status).toBe(200)
    const since = await fake.cursor()
    const response = await s.api.post<{ deletedCount: number; whatsappFlows: unknown[] }>("/v1/mini-apps/bulk-delete", {
      ids: [s.app.id],
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ deletedCount: 1, whatsappFlows: [] })
    expect(await upstreamCalls({ host: GRAPH, since, bearer: s.accessToken })).toHaveLength(0)
  })
})
