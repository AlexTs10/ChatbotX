// flows.export / flows.import (scope `automation`). export: the published
// graph (or `version=draft`) as a portable document (formatVersion 2).
// import: a flow-import file uploaded with the session upload route
// (`POST /api/presigned-upload`, then a PUT to the presigned URL) is queued
// (202 {importId}); the worker creates a draft-only flow from it. The import
// job status has no public read route, so the result is checked through
// flows.list / flows.get.

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { signedInClient, type Tenant } from "../../src/fixtures/index.ts"
import { Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  createFlow,
  createPublishedFlow,
  draftOf,
  expectError,
  expectInvalid,
  ext,
  type FlowDetail,
  getFlow,
  newTenant,
  textSpec,
  uniqueName,
} from "./helpers.ts"

const UNKNOWN_ID = "123456789012345"

type ExportDoc = {
  formatVersion: number
  exportedAt: string
  source: { workspaceId: string; flowId: string }
  flows: Array<{
    name: string
    active: boolean
    enableInInbox: boolean
    startNodeId: string
    nodes: unknown[]
    edges: unknown[]
  }>
  customFields: Record<string, unknown>
  botFields: Record<string, unknown>
}

const exportFlow = async (api: Api, id: string, query = ""): Promise<{ fileName: string; export: ExportDoc }> => {
  const response = await api.get<{ fileName: string; export: ExportDoc }>(`/v1/flows/${id}/export${query}`)
  expect(response.status, response.text).toBe(200)
  expect(Object.keys(response.json).sort()).toEqual(["export", "fileName"])
  return response.json
}

/** Uploads `content` as a flow-import file through the session upload route; returns the file id. */
const uploadFlowFile = async (tenant: Tenant, content: string): Promise<string> => {
  const http = await signedInClient(tenant)
  const presign = await http.post<{ fileId: string; presignedPostUrl: string; path: string }>("/api/presigned-upload", {
    type: "import",
    subType: "flow",
    workspaceId: tenant.workspaceId,
    fileName: "flow.json",
    mimeType: "application/json",
  })
  expect(presign.status, presign.text).toBe(200)
  expect(presign.json.path).toMatch(new RegExp(`^workspaces/${tenant.workspaceId}/imports/flows/import_\\d+\\.json$`))
  const put = await fetch(presign.json.presignedPostUrl, {
    method: "PUT",
    body: content,
    headers: { "content-type": "application/json" },
  })
  expect(put.status, await put.text()).toBe(200)
  return presign.json.fileId
}

const startImport = async (api: Api, body: Record<string, unknown>): Promise<string> => {
  const response = await api.post<{ importId: string }>("/v1/flows/import", body)
  expect(response.status, response.text).toBe(202)
  expect(Object.keys(response.json)).toEqual(["importId"])
  expect(response.json.importId).toMatch(/^\d+$/)
  return response.json.importId
}

/** Waits until a flow named `name` (other than `exclude`) is listed; returns its detail. */
const importedFlow = async (api: Api, name: string, exclude: string[] = []): Promise<FlowDetail> =>
  eventually(
    async () => {
      for (const active of ["true", "false"]) {
        const list = await api.get<{ data: Array<{ id: string; name: string }> }>(`/v1/flows?active=${active}`)
        const found = list.json.data.find((flow) => flow.name === name && !exclude.includes(flow.id))
        if (found) {
          return await getFlow(api, found.id)
        }
      }
      throw new Error(`no imported flow named ${name} yet`)
    },
    { timeoutMs: 45_000, intervalMs: 500, description: "imported flow" },
  )

describe("flows.export", () => {
  it("exports the published graph by default; version=draft exports the draft", async () => {
    const { tenant, api } = await newTenant("p109-export")
    const name = `Export Me! ${uniqueName("x")}`
    const id = await createPublishedFlow(api, textSpec(name, "published text"))
    expect((await api.put(`/v1/flows/${id}/draft`, { spec: textSpec(name, "draft text") })).status).toBe(204)
    const flow = await getFlow(api, id)
    const published = flow.flowVersions.find((version) => !version.isDraft)
    const draft = draftOf(flow)

    const exported = await exportFlow(api, id)
    const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "")
    expect(exported.fileName).toBe(`${slug}.chatbotx-flow.json`)
    const n = new Normalizer()
    expect(n.normalize(exported.export, { keep: ["nodes", "edges", "startNodeId"] })).toEqual({
      formatVersion: 2,
      exportedAt: "<ts>",
      source: { workspaceId: n.idLabel(tenant.workspaceId), flowId: n.idLabel(id) },
      flows: [
        {
          name,
          active: true,
          enableInInbox: false,
          startNodeId: published?.startNodeId,
          nodes: published?.nodes,
          edges: published?.edges,
        },
      ],
      customFields: {},
      botFields: {},
    })

    const draftExport = await exportFlow(api, id, "?version=draft")
    expect(draftExport.export.flows[0]).toMatchObject({ startNodeId: draft.startNodeId, nodes: draft.nodes, edges: draft.edges })
    expect((await exportFlow(api, id, "?version=published")).export.flows[0]?.nodes).toEqual(published?.nodes)
  })

  it("lists the custom fields the graph uses by id with name and type", async () => {
    const { api } = await newTenant("p109-export-fields")
    const field = await api.post<{ id: string }>("/v1/custom-fields", { name: "Shoe size", type: "number" })
    expect([200, 201], field.text).toContain(field.status)
    const id = await createPublishedFlow(api, {
      formatVersion: 1,
      name: "Fields",
      steps: [{ type: "action", action: "setCustomField", customFieldName: "Shoe size", value: "42" }],
    })
    const exported = await exportFlow(api, id)
    expect(exported.export.customFields).toEqual({ [field.json.id]: { name: "Shoe size", type: "number" } })
    expect(exported.export.botFields).toEqual({})
  })

  it("errors: never published 409 flowNotPublished, unknown / foreign 404, bad version 422", async () => {
    const { api } = await newTenant("p109-export-bad")
    const { api: otherApi } = await newTenant("p109-export-other")
    const draftOnly = await createFlow(api, { name: "Draft" })
    expectError(await api.get(`/v1/flows/${draftOnly.id}/export`), 409, "flowNotPublished", {
      defined: false,
      message: "The flow has no published version to export",
    })
    // The draft of a never-published flow exports.
    expect((await exportFlow(api, draftOnly.id, "?version=draft")).export.flows[0]?.name).toBe("Draft")
    const foreign = await createPublishedFlow(otherApi, textSpec("Foreign", "x"))
    expectError(await api.get(`/v1/flows/${UNKNOWN_ID}/export`), 404, "notFound", { message: "Flow not found" })
    expectError(await api.get(`/v1/flows/${foreign}/export`), 404, "notFound")
    expectInvalid(await api.get(`/v1/flows/${draftOnly.id}/export?version=latest`), "version")
  })
})

describe("flows.import", () => {
  it("imports an export into another workspace as a draft-only flow with the same graph", async () => {
    const { api: sourceApi } = await newTenant("p109-import-source")
    const name = uniqueName("Imported")
    const sourceId = await createPublishedFlow(sourceApi, textSpec(name, "carried over", "and more"))
    const exported = await exportFlow(sourceApi, sourceId)

    const { tenant, api } = await newTenant("p109-import-target")
    const fileId = await uploadFlowFile(tenant, JSON.stringify(exported.export))
    await startImport(api, { fileId, folderId: null })
    const flow = await importedFlow(api, name)
    expect(flow).toMatchObject({
      name,
      workspaceId: tenant.workspaceId,
      folderId: null,
      active: true,
      enableInInbox: false,
      currentVersionId: null,
    })
    expect(flow.flowVersions).toHaveLength(1)
    const draft = draftOf(flow)
    const exportedFlow = exported.export.flows[0]
    // Node and step ids are reused as they are.
    expect(draft).toMatchObject({ startNodeId: exportedFlow?.startNodeId, nodes: exportedFlow?.nodes, edges: exportedFlow?.edges })
  })

  it("imports into a flow folder of the workspace; an unknown folder id falls back to no folder", async () => {
    const { tenant, api } = await newTenant("p109-import-folder")
    const name = uniqueName("Foldered")
    const sourceId = await createPublishedFlow(api, textSpec(name, "x"))
    const doc = JSON.stringify((await exportFlow(api, sourceId)).export)
    const { folderId } = await (await ext()).createFolder({ workspaceId: tenant.workspaceId, name: "Imports", folderType: "flow" })

    await startImport(api, { fileId: await uploadFlowFile(tenant, doc), folderId })
    const inFolder = await importedFlow(api, name, [sourceId])
    expect(inFolder.folderId).toBe(folderId)

    await startImport(api, { fileId: await uploadFlowFile(tenant, doc), folderId: UNKNOWN_ID })
    const noFolder = await importedFlow(api, name, [sourceId, inFolder.id])
    expect(noFolder.folderId).toBeNull()
  })

  it("errors: unknown / foreign file -> 422 validation 'File not found'; a non-flow import file -> 'File is not a flow import'", async () => {
    const { tenant, api } = await newTenant("p109-import-bad")
    const { tenant: other } = await newTenant("p109-import-other")
    expectError(await api.post("/v1/flows/import", { fileId: UNKNOWN_ID, folderId: null }), 422, "validation", {
      defined: true,
      message: "File not found",
    })
    const foreignFile = await uploadFlowFile(other, "{}")
    expectError(await api.post("/v1/flows/import", { fileId: foreignFile, folderId: null }), 422, "validation", {
      message: "File not found",
    })
    const contactsUpload = await api.post<{ fileId: string }>("/v1/contacts/imports/upload-url", {
      fileName: "c.csv",
      mimeType: "text/csv",
      fileSize: 10,
    })
    expect(contactsUpload.status, contactsUpload.text).toBe(201)
    expectError(await api.post("/v1/flows/import", { fileId: contactsUpload.json.fileId, folderId: null }), 422, "validation", {
      defined: true,
      message: "File is not a flow import",
    })
    expectInvalid(await api.post("/v1/flows/import", {}), "fileId")
    expectInvalid(await api.post("/v1/flows/import", { fileId: "abc", folderId: null }), "fileId")
    expectInvalid(await api.post("/v1/flows/import", { fileId: UNKNOWN_ID }), "folderId")
    void tenant
  })
})
