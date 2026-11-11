// flows.publish / updateDraft / versions / listNodes (scope `automation`).
// publish: compiles + validates the graph, rewrites the draft and adds an
// immutable published version (the new `currentVersionId`). updateDraft:
// rewrites the draft only (not validated). versions: published versions,
// latest first. listNodes: the nodes of the published version.

import { describe, expect, it } from "vitest"
import {
  type Api,
  createFlow,
  createPublishedFlow,
  draftOf,
  expectAuthoringErrors,
  expectError,
  expectInvalid,
  type FlowVersion,
  getFlow,
  newTenant,
  sendTexts,
  textSpec,
  uniqueName,
} from "./helpers.ts"

const UNKNOWN_ID = "123456789012345"
const VERSION_KEYS = ["createdAt", "edges", "flowId", "id", "isDraft", "isLatest", "nodes", "startNodeId", "workspaceId"]

const versions = async (api: Api, id: string): Promise<FlowVersion[]> => {
  const response = await api.get<{ data: FlowVersion[] }>(`/v1/flows/${id}/versions`)
  expect(response.status, response.text).toBe(200)
  expect(Object.keys(response.json)).toEqual(["data"])
  return response.json.data
}

const publish = async (api: Api, id: string, body: Record<string, unknown>) => {
  const response = await api.post(`/v1/flows/${id}/publish`, body)
  expect(response.status, response.text).toBe(204)
  expect(response.text).toBe("")
}

describe("flows.publish + flows.versions", () => {
  it("publishes a spec: new version, draft synced, versions latest first", async () => {
    const { tenant, api } = await newTenant("p109-flow-publish")
    const name = uniqueName("Pub")
    const created = await createFlow(api, { name })
    const originalDraft = draftOf(await getFlow(api, created.id))
    expect(await versions(api, created.id)).toEqual([])

    await publish(api, created.id, { spec: textSpec(name, "first", "second") })
    const afterFirst = await getFlow(api, created.id)
    const [v1] = await versions(api, created.id)
    expect(Object.keys(v1 ?? {}).sort()).toEqual(VERSION_KEYS)
    expect(v1).toMatchObject({
      flowId: created.id,
      workspaceId: tenant.workspaceId,
      isDraft: false,
      isLatest: true,
      // The published version keeps the draft's original startNodeId, even
      // though the spec replaced every node (original behavior).
      startNodeId: originalDraft.startNodeId,
    })
    expect(sendTexts(v1 as FlowVersion)).toEqual(["first", "second"])
    expect((v1 as FlowVersion).nodes.map((node) => node.id)).not.toContain(originalDraft.startNodeId)
    expect(afterFirst.currentVersionId).toBe(v1?.id)
    const draft = draftOf(afterFirst)
    expect(draft).toMatchObject({ id: originalDraft.id, isDraft: true, isLatest: false, startNodeId: originalDraft.startNodeId })
    expect(draft.nodes).toEqual(v1?.nodes)
    expect(draft.edges).toEqual(v1?.edges)

    await publish(api, created.id, { spec: textSpec(name, "third") })
    const list = await versions(api, created.id)
    expect(list).toHaveLength(2)
    const [latest, older] = list as [FlowVersion, FlowVersion]
    expect(latest).toMatchObject({ isLatest: true, isDraft: false })
    expect(sendTexts(latest)).toEqual(["third"])
    expect(older).toEqual({ ...(v1 as FlowVersion), isLatest: false })
    expect(Date.parse(latest.createdAt)).toBeGreaterThanOrEqual(Date.parse(older.createdAt))
    const afterSecond = await getFlow(api, created.id)
    expect(afterSecond.currentVersionId).toBe(latest.id)
    expect(afterSecond.flowVersions).toHaveLength(3)
    expect(sendTexts(draftOf(afterSecond))).toEqual(["third"])
  })

  it("publishes a raw {nodes, edges} graph (the shape flows.validate returns)", async () => {
    const { api } = await newTenant("p109-flow-publish-raw")
    const name = uniqueName("Raw")
    const created = await createFlow(api, { name })
    const compiled = await api.post<{ nodes: unknown[]; edges: unknown[] }>("/v1/flows/validate", {
      spec: textSpec(name, "raw graph"),
    })
    expect(compiled.status).toBe(200)
    await publish(api, created.id, compiled.json)
    const [version] = await versions(api, created.id)
    expect(version?.nodes).toEqual(compiled.json.nodes)
    expect(version?.edges).toEqual(compiled.json.edges)
  })

  it("rejects an invalid graph (422) and keeps the flow unpublished", async () => {
    const { api } = await newTenant("p109-flow-publish-bad")
    const name = uniqueName("Bad")
    const created = await createFlow(api, { name })
    const before = await getFlow(api, created.id)
    const threads = await api.post(`/v1/flows/${created.id}/publish`, {
      spec: { formatVersion: 1, name, channel: "threads", steps: [{ type: "send", text: "x" }] },
    })
    expect(expectAuthoringErrors(threads).map((error) => [error.path, error.code])).toEqual([
      ["steps[0].data.details.steps[0]", "unsupportedBlock"],
    ])
    const unknownFlow = await api.post(`/v1/flows/${created.id}/publish`, {
      spec: { formatVersion: 1, name, steps: [{ type: "startFlow", flowName: name.slice(0, -2) }] },
    })
    expect(expectAuthoringErrors(unknownFlow)).toEqual([
      {
        path: "steps[0].flowName",
        code: "unknownFlow",
        message: `No flow named "${name.slice(0, -2)}" in this workspace.`,
        hint: "Call flows.list and pick a name from the results.",
        candidates: [name],
      },
    ])
    const raw = await api.post(`/v1/flows/${created.id}/publish`, { nodes: [{ id: "x" }], edges: [] })
    expectError(raw, 422, "invalidRequestData", { defined: true })
    expectInvalid(await api.post(`/v1/flows/${created.id}/publish`, {}), "")
    expectInvalid(
      await api.post(`/v1/flows/${created.id}/publish`, { spec: textSpec(name, "x"), nodes: [], edges: [] }),
      "",
    )
    expect(await getFlow(api, created.id)).toEqual(before)
    expect(await versions(api, created.id)).toEqual([])
  })

  it("unknown or foreign flow -> 404 notFound (after the graph is compiled)", async () => {
    const { api } = await newTenant("p109-flow-publish-404")
    const { api: otherApi } = await newTenant("p109-flow-publish-other")
    const foreign = await createFlow(otherApi, { name: "Foreign" })
    const spec = textSpec("x", "x")
    expectError(await api.post(`/v1/flows/${UNKNOWN_ID}/publish`, { spec }), 404, "notFound", { message: "Flow not found" })
    expectError(await api.post(`/v1/flows/${foreign.id}/publish`, { spec }), 404, "notFound")
    expect((await getFlow(otherApi, foreign.id)).currentVersionId).toBeNull()
    expectInvalid(await api.post("/v1/flows/abc/publish", { spec }), "id")
  })

  it("versions of a draft-only flow or of an id that never existed: empty list (200)", async () => {
    const { api } = await newTenant("p109-flow-versions")
    const created = await createFlow(api, { name: "Draft" })
    expect(await versions(api, created.id)).toEqual([])
    expect(await versions(api, UNKNOWN_ID)).toEqual([])
    expectInvalid(await api.get("/v1/flows/abc/versions"), "id")
  })
})

describe("flows.updateDraft", () => {
  it("overwrites the draft from a spec (204); the published version is not touched", async () => {
    const { api } = await newTenant("p109-flow-draft")
    const name = uniqueName("Draft")
    const id = await createPublishedFlow(api, textSpec(name, "live"))
    const before = await getFlow(api, id)
    const response = await api.put(`/v1/flows/${id}/draft`, { spec: textSpec(name, "next", "after") })
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    const after = await getFlow(api, id)
    const draft = draftOf(after)
    expect(sendTexts(draft)).toEqual(["next", "after"])
    expect(draft.startNodeId).toBe(draftOf(before).startNodeId)
    expect(after.currentVersionId).toBe(before.currentVersionId)
    const published = after.flowVersions.find((version) => !version.isDraft)
    expect(sendTexts(published as FlowVersion)).toEqual(["live"])
    expect(await versions(api, id)).toHaveLength(1)
  })

  it("stores a raw graph verbatim, without publish validation", async () => {
    const { api } = await newTenant("p109-flow-draft-raw")
    const created = await createFlow(api, { name: "Raw draft" })
    const nodes = [
      { id: "1001", type: "sendMessage", position: { x: 1, y: 2 }, data: { name: "Hand made", anything: true } },
      { id: "1002", type: "sendMessage", position: { x: 3, y: 4 }, data: { name: "Second" } },
    ]
    const edges = [{ id: "2001", source: "1001", target: "1002", sourceHandle: "1001", targetHandle: "1002" }]
    const response = await api.put(`/v1/flows/${created.id}/draft`, { nodes, edges })
    expect(response.status, response.text).toBe(204)
    const draft = draftOf(await getFlow(api, created.id))
    expect(draft.nodes).toEqual(nodes)
    expect(draft.edges).toEqual(edges)
    // Publishing that draft graph as it is fails validation.
    const publishResponse = await api.post(`/v1/flows/${created.id}/publish`, { nodes, edges })
    expectError(publishResponse, 422, "invalidRequestData")
  })

  it("errors: unknown / foreign flow 404, mixed or missing body 422", async () => {
    const { api } = await newTenant("p109-flow-draft-bad")
    const { api: otherApi } = await newTenant("p109-flow-draft-other")
    const foreign = await createFlow(otherApi, { name: "Foreign" })
    const spec = textSpec("x", "x")
    expectError(await api.put(`/v1/flows/${UNKNOWN_ID}/draft`, { spec }), 404, "notFound", {
      message: "Draft flow version not found",
    })
    expectError(await api.put(`/v1/flows/${foreign.id}/draft`, { spec }), 404, "notFound")
    const own = await createFlow(api, { name: "Own" })
    expectInvalid(await api.put(`/v1/flows/${own.id}/draft`, { spec, nodes: [], edges: [] }), "")
    expectInvalid(await api.put(`/v1/flows/${own.id}/draft`, {}), "")
    expectInvalid(await api.put(`/v1/flows/${own.id}/draft`, { nodes: [] }), "")
  })
})

describe("flows.listNodes", () => {
  it("lists the published version's nodes {flowId,nodeId,label,nodeType} in graph order", async () => {
    const { api } = await newTenant("p109-flow-nodes")
    const name = uniqueName("Nodes")
    const id = await createPublishedFlow(api, {
      formatVersion: 1,
      name,
      steps: [
        { type: "send", text: "hello" },
        { type: "action", action: "addTags", tagNames: ["p109"] },
        { type: "addNote", note: "internal" },
      ],
    })
    const published = (await getFlow(api, id)).flowVersions.find((version) => !version.isDraft) as FlowVersion
    const response = await api.get<{ data: Array<Record<string, unknown>> }>(`/v1/flows/${id}/nodes`)
    expect(response.status, response.text).toBe(200)
    expect(response.json.data).toEqual(
      published.nodes.map((node) => ({
        flowId: id,
        nodeId: node.id,
        label: (node.data as { name: string }).name,
        nodeType: node.type,
      })),
    )
    expect(response.json.data.map((node) => node.nodeType)).toEqual(["sendMessage", "performAction", "addNotes"])
    expect(response.json.data.map((node) => node.label)).toEqual(
      expect.arrayContaining(["Send Message", "Perform Action"]),
    )

    // A new draft does not change the list until it is published.
    expect((await api.put(`/v1/flows/${id}/draft`, { spec: textSpec(name, "draft") })).status).toBe(204)
    const again = await api.get<{ data: unknown[] }>(`/v1/flows/${id}/nodes`)
    expect(again.json).toEqual(response.json)
  })

  it("a never-published flow has no nodes; an unknown or foreign flow -> 404", async () => {
    const { api } = await newTenant("p109-flow-nodes-empty")
    const { api: otherApi } = await newTenant("p109-flow-nodes-other")
    const created = await createFlow(api, { name: "Draft" })
    const response = await api.get(`/v1/flows/${created.id}/nodes`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ data: [] })
    const foreign = await createPublishedFlow(otherApi, textSpec("Foreign", "x"))
    expectError(await api.get(`/v1/flows/${UNKNOWN_ID}/nodes`), 404, "notFound", { message: "Flow does not exists." })
    expectError(await api.get(`/v1/flows/${foreign}/nodes`), 404, "notFound")
  })
})
