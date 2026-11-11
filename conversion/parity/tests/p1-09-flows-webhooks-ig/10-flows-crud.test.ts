// flows.create / get / list / update / delete / duplicate (scope
// `automation`). Shapes recorded on the original stack: a flow row plus all
// its versions (`flowVersions`, not ordered: compared by `isDraft`), the
// default draft (one empty "Send Message #1" start node), spec-compiled
// graphs, raw graphs with server-side node ids.

import { describe, expect, it } from "vitest"
import { Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  createFlow,
  createPublishedFlow,
  draftOf,
  expectAuthoringErrors,
  expectError,
  expectInvalid,
  ext,
  type FlowDetail,
  getFlow,
  newTenant,
  nodeSteps,
  sendTexts,
  textSpec,
  uniqueName,
} from "./helpers.ts"

const UNKNOWN_ID = "123456789012345"
const FLOW_KEYS = [
  "active",
  "createdAt",
  "currentVersionId",
  "draftVersionId",
  "enableInInbox",
  "flowVersions",
  "folderId",
  "id",
  "name",
  "updatedAt",
  "welcomeMessageFlows",
  "workspaceId",
]
const VERSION_KEYS = ["createdAt", "edges", "flowId", "id", "isDraft", "isLatest", "nodes", "startNodeId", "workspaceId"]

type ListItem = { id: string; name: string; folderId: string | null; active: boolean; enableInInbox: boolean }
type ListPage = { data: ListItem[]; pageCount: number }

const list = async (api: Api, query = ""): Promise<ListPage> => {
  const response = await api.get<ListPage>(`/v1/flows${query}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

/** The quick-reply settings and channel step every compiled send node carries. */
const sendNodeDetails = (steps: unknown[], channel = "omnichannel") => ({
  beforeStep: { id: expect.stringMatching(/^\d+$/), stepType: "chooseChannel", channel },
  steps,
  quickReplies: [],
  quickReplySettings: {
    retry: { id: expect.stringMatching(/^\d+$/), target: null, enabled: false, message: "", maxRetries: 3 },
    followUp: { id: expect.stringMatching(/^\d+$/), unit: "days", target: null, enabled: false, duration: 1 },
  },
})

describe("flows.create", () => {
  it("without content: a draft-only flow with one empty start node (201 {id})", async () => {
    const { tenant, api } = await newTenant("p109-flow-create")
    const name = uniqueName("Flow")
    const response = await api.post<{ id: string }>("/v1/flows", { name: ` ${name} `, folderId: null })
    expect(response.status, response.text).toBe(201)
    expect(Object.keys(response.json)).toEqual(["id"])

    const flow = await getFlow(api, response.json.id)
    expect(Object.keys(flow).sort()).toEqual(FLOW_KEYS)
    const n = new Normalizer()
    const replace = { [tenant.workspaceId]: "<workspace>", [flow.id]: "<flow>" }
    expect(n.normalize({ ...flow, flowVersions: [] }, { replace })).toEqual({
      id: "<flow>",
      name,
      workspaceId: "<workspace>",
      folderId: null,
      active: true,
      enableInInbox: true,
      currentVersionId: null,
      // Not set by flows.create (the draft is found by `isDraft`).
      draftVersionId: null,
      createdAt: "<ts>",
      updatedAt: "<ts>",
      flowVersions: [],
      welcomeMessageFlows: [],
    })
    expect(flow.flowVersions).toHaveLength(1)
    const draft = draftOf(flow)
    expect(Object.keys(draft).sort()).toEqual(VERSION_KEYS)
    expect(draft).toMatchObject({ flowId: flow.id, workspaceId: tenant.workspaceId, isDraft: true, isLatest: false })
    expect(draft.edges).toEqual([])
    expect(draft.nodes).toEqual([
      {
        id: draft.startNodeId,
        type: "sendMessage",
        position: { x: 100, y: 300 },
        measured: { width: 288, height: 100 },
        data: { name: "Send Message #1", isStartNode: true, details: sendNodeDetails([]) },
      },
    ])
  })

  it("with a spec: the compiled draft, not published; no nodeIds in the response", async () => {
    const { api } = await newTenant("p109-flow-create-spec")
    const name = uniqueName("Spec")
    const response = await api.post<Record<string, unknown>>("/v1/flows", {
      name,
      folderId: null,
      spec: textSpec(name, "one", "two"),
    })
    expect(response.status, response.text).toBe(201)
    expect(Object.keys(response.json)).toEqual(["id"])
    const flow = await getFlow(api, response.json.id as string)
    expect(flow).toMatchObject({ currentVersionId: null, active: true, enableInInbox: true })
    expect(flow.flowVersions).toHaveLength(1)
    const draft = draftOf(flow)
    expect(sendTexts(draft)).toEqual(["one", "two"])
    const [first, second] = draft.nodes
    expect(draft.startNodeId).toBe(first?.id)
    expect(first).toEqual({
      id: expect.stringMatching(/^\d+$/),
      type: "sendMessage",
      position: { x: 100, y: 100 },
      measured: { width: 288, height: 100 },
      data: {
        name: "Send Message",
        isStartNode: true,
        details: sendNodeDetails([{ id: expect.stringMatching(/^\d+$/), stepType: "sendText", text: "one", buttons: [] }]),
      },
    })
    expect(second).toMatchObject({ type: "sendMessage", position: { x: 508, y: 100 }, data: { isStartNode: false } })
    expect(draft.edges).toEqual([
      {
        id: expect.stringMatching(/^\d+$/),
        source: first?.id,
        sourceHandle: first?.id,
        target: second?.id,
        targetHandle: second?.id,
      },
    ])
  })

  it("with a spec and publish: true: draft + published version, enableInInbox false", async () => {
    const { api } = await newTenant("p109-flow-create-pub")
    const name = uniqueName("Pub")
    const id = await createPublishedFlow(api, textSpec(name, "hello"))
    const flow = await getFlow(api, id)
    expect(flow.flowVersions).toHaveLength(2)
    const draft = draftOf(flow)
    const published = flow.flowVersions.find((version) => !version.isDraft)
    expect(published).toMatchObject({ isDraft: false, isLatest: true, startNodeId: draft.startNodeId })
    expect(flow).toMatchObject({
      active: true,
      // The published create path writes enableInInbox false and the draft id.
      enableInInbox: false,
      currentVersionId: published?.id,
      draftVersionId: draft.id,
    })
    expect(published?.nodes).toEqual(draft.nodes)
    expect(published?.edges).toEqual(draft.edges)
    expect(sendTexts(draft)).toEqual(["hello"])
  })

  it("with raw nodes/edges: node ids are remapped (nodeIds), step ids and data kept, layout generated", async () => {
    const { api } = await newTenant("p109-flow-create-raw")
    const response = await createFlow(api, {
      name: uniqueName("Raw"),
      nodes: [
        {
          id: "a",
          type: "sendMessage",
          data: { name: "A", isStartNode: true, details: { steps: [{ id: "s1", stepType: "sendText", text: "raw a", buttons: [] }] } },
        },
        {
          id: "b",
          type: "sendMessage",
          data: { name: "B", details: { steps: [{ id: "s2", stepType: "sendText", text: "raw b", buttons: [] }] } },
        },
      ],
      edges: [{ source: "a", target: "b" }],
    })
    expect(Object.keys(response).sort()).toEqual(["id", "nodeIds"])
    const nodeIds = response.nodeIds as Record<string, string>
    expect(Object.keys(nodeIds).sort()).toEqual(["a", "b"])
    expect(nodeIds.a).toMatch(/^\d+$/)
    expect(nodeIds.b).toMatch(/^\d+$/)
    const draft = draftOf(await getFlow(api, response.id))
    expect(draft.startNodeId).toBe(nodeIds.a)
    expect(draft.nodes).toEqual([
      {
        id: nodeIds.a,
        type: "sendMessage",
        position: { x: 100, y: 100 },
        measured: { width: 288, height: 100 },
        data: { name: "A", isStartNode: true, details: { steps: [{ id: "s1", stepType: "sendText", text: "raw a", buttons: [] }] } },
      },
      {
        id: nodeIds.b,
        type: "sendMessage",
        position: { x: 508, y: 100 },
        measured: { width: 288, height: 100 },
        data: { name: "B", isStartNode: false, details: { steps: [{ id: "s2", stepType: "sendText", text: "raw b", buttons: [] }] } },
      },
    ])
    expect(draft.edges).toEqual([
      { id: expect.stringMatching(/^\d+$/), source: nodeIds.a, sourceHandle: nodeIds.a, target: nodeIds.b, targetHandle: nodeIds.b },
    ])
  })

  it("publish: true validates the graph first: 422 with authoring errors and no flow row", async () => {
    const { api } = await newTenant("p109-flow-create-invalid")
    const name = uniqueName("Bad")
    const raw = await api.post("/v1/flows", {
      name,
      folderId: null,
      publish: true,
      nodes: [
        {
          id: "a",
          type: "sendMessage",
          data: { name: "A", isStartNode: true, details: { steps: [{ id: "s1", stepType: "sendText", text: "x", buttons: [] }] } },
        },
      ],
    })
    const rawErrors = expectAuthoringErrors(raw)
    expect((raw.json as { defined: boolean }).defined).toBe(false)
    expect(rawErrors).toEqual([
      {
        path: "nodes[0].data.details.beforeStep",
        code: "invalidStep",
        message: "Invalid input: expected object, received undefined",
      },
      { path: "nodes[0].data.details.steps[0].id", code: "invalidStep", message: "Invalid string: must match pattern /^\\d+$/" },
      {
        path: "nodes[0].data.details.quickReplies",
        code: "invalidStep",
        message: "Invalid input: expected array, received undefined",
      },
    ])
    const spec = await api.post("/v1/flows", {
      name,
      folderId: null,
      publish: true,
      spec: { formatVersion: 1, name, channel: "threads", steps: [{ type: "send", text: "x" }] },
    })
    expect(expectAuthoringErrors(spec)).toEqual([
      {
        capability: { block: "sendText", channel: "threads", code: "unsupportedBlock", policyVersion: 3 },
        path: "steps[0].data.details.steps[0]",
        code: "unsupportedBlock",
        message: "The threads channel does not support sendText.",
      },
    ])
    expect(await list(api)).toEqual({ data: [], pageCount: 0 })
  })

  it("validates the body (422 invalidRequestData)", async () => {
    const { api } = await newTenant("p109-flow-create-bad")
    const name = uniqueName("Bad")
    expectInvalid(await api.post("/v1/flows", { folderId: null }), "name")
    expectInvalid(await api.post("/v1/flows", { name: "   ", folderId: null }), "name")
    expectInvalid(await api.post("/v1/flows", { name: "x".repeat(256), folderId: null }), "name")
    expectInvalid(await api.post("/v1/flows", { name }), "folderId")
    expectInvalid(await api.post("/v1/flows", { name, folderId: "abc" }), "folderId")
    const mixed = await api.post("/v1/flows", { name, folderId: null, spec: textSpec(name, "x"), nodes: [{ id: "a" }] })
    expectInvalid(mixed, "spec")
    expect(JSON.stringify(mixed.json)).toContain('Cannot combine \\"spec\\" with \\"nodes\\"/\\"edges\\" in the same request body.')
    expectInvalid(await api.post("/v1/flows", { name, folderId: null, edges: [] }), "edges")
    expectInvalid(await api.post("/v1/flows", { name, folderId: null, publish: true }), "publish")
    expectInvalid(await api.post("/v1/flows", { name, folderId: null, nodes: [] }), "nodes")
    expectInvalid(await api.post("/v1/flows", { name, folderId: null, spec: { name, steps: [] } }), "spec.formatVersion")
    expect(await list(api)).toEqual({ data: [], pageCount: 0 })
  })

  it("folderId must be a flow folder of the workspace (else 404 notFound, defined false)", async () => {
    const { tenant, api } = await newTenant("p109-flow-create-folder")
    const name = uniqueName("Folder")
    const { folderId } = await (await ext()).createFolder({ workspaceId: tenant.workspaceId, name: "Flows", folderType: "flow" })
    const tagFolder = await api.post<{ id: string }>("/v1/folders", { name: "Tags", folderType: "tag" })
    expectError(await api.post("/v1/flows", { name, folderId: UNKNOWN_ID }), 404, "notFound", {
      defined: false,
      message: "Folder not found",
    })
    expectError(await api.post("/v1/flows", { name, folderId: tagFolder.json.id }), 404, "notFound", { defined: false })
    const created = await createFlow(api, { name, folderId })
    expect((await getFlow(api, created.id)).folderId).toBe(folderId)
  })
})

describe("flows.get", () => {
  it("unknown id, another workspace's flow -> 404 notFound; a non-numeric id -> 422", async () => {
    const { api } = await newTenant("p109-flow-get")
    const { api: otherApi } = await newTenant("p109-flow-get-other")
    const foreign = await createFlow(otherApi, { name: uniqueName("Foreign") })
    expectError(await api.get(`/v1/flows/${UNKNOWN_ID}`), 404, "notFound", { defined: true, message: "Flow does not exists." })
    expectError(await api.get(`/v1/flows/${foreign.id}`), 404, "notFound")
    expectInvalid(await api.get("/v1/flows/abc"), "id")
  })
})

describe("flows.list", () => {
  it("lists active flows newest first as {id,name,folderId,active,enableInInbox} with pageCount", async () => {
    const { tenant, api } = await newTenant("p109-flow-list")
    const { api: otherApi } = await newTenant("p109-flow-list-other")
    await createFlow(otherApi, { name: "Not mine" })
    const { folderId } = await (await ext()).createFolder({ workspaceId: tenant.workspaceId, name: "Flows", folderType: "flow" })
    const first = await createFlow(api, { name: "Bravo first" })
    const second = await createPublishedFlow(api, textSpec("Alpha second", "x"))
    const third = await createFlow(api, { name: "Charlie third", folderId })

    const page = await list(api)
    expect(page).toEqual({
      data: [
        { id: third.id, name: "Charlie third", folderId, active: true, enableInInbox: true },
        { id: second, name: "Alpha second", folderId: null, active: true, enableInInbox: false },
        { id: first.id, name: "Bravo first", folderId: null, active: true, enableInInbox: true },
      ],
      pageCount: 1,
    })

    // Filters: name (case-insensitive substring), folderId ("0" = no folder), enableInInbox.
    expect((await list(api, "?name=ALPHA")).data.map((flow) => flow.id)).toEqual([second])
    expect((await list(api, "?name=ir")).data.map((flow) => flow.id)).toEqual([third.id, first.id])
    expect((await list(api, `?folderId=${folderId}`)).data.map((flow) => flow.id)).toEqual([third.id])
    expect((await list(api, "?folderId=0")).data.map((flow) => flow.id)).toEqual([second, first.id])
    expect((await list(api, "?enableInInbox=false")).data.map((flow) => flow.id)).toEqual([second])
    expect((await list(api, "?enableInInbox=true")).data.map((flow) => flow.id)).toEqual([third.id, first.id])

    // Sort and paging.
    expect((await list(api, "?sort[0][id]=name&sort[0][desc]=false")).data.map((flow) => flow.name)).toEqual([
      "Alpha second",
      "Bravo first",
      "Charlie third",
    ])
    expect((await list(api, "?sort[0][id]=createdAt&sort[0][desc]=false")).data.map((flow) => flow.id)).toEqual([
      first.id,
      second,
      third.id,
    ])
    expect(await list(api, "?perPage=2&page=2")).toEqual({
      data: [{ id: first.id, name: "Bravo first", folderId: null, active: true, enableInInbox: true }],
      pageCount: 2,
    })
    expect(await list(api, "?perPage=2&page=3")).toEqual({ data: [], pageCount: 2 })
  })

  it("active=false lists only the inactive flows (not 'all flows')", async () => {
    const { api } = await newTenant("p109-flow-list-active")
    const on = await createFlow(api, { name: "On" })
    const off = await createFlow(api, { name: "Off" })
    expect((await api.patch(`/v1/flows/${off.id}`, { active: false })).status).toBe(204)
    expect((await list(api)).data.map((flow) => flow.id)).toEqual([on.id])
    expect((await list(api, "?active=true")).data.map((flow) => flow.id)).toEqual([on.id])
    expect(await list(api, "?active=false")).toEqual({
      data: [{ id: off.id, name: "Off", folderId: null, active: false, enableInInbox: true }],
      pageCount: 1,
    })
  })

  it("startType: sendWaTemplateMessage needs integrationWhatsappId (else empty); an unknown channel -> 404", async () => {
    const { api } = await newTenant("p109-flow-list-start")
    await createPublishedFlow(api, textSpec("Text only", "x"))
    expect(await list(api, "?startType=sendWaTemplateMessage")).toEqual({ data: [], pageCount: 0 })
    expect(await list(api, "?startType=sendMessengerTemplateMessage")).toEqual({ data: [], pageCount: 0 })
    expectError(
      await api.get(`/v1/flows?startType=sendWaTemplateMessage&integrationWhatsappId=${UNKNOWN_ID}`),
      404,
      "notFound",
      { message: "WhatsApp channel not found" },
    )
  })

  it("an empty workspace: no data, pageCount 0; bad query -> 422", async () => {
    const { api } = await newTenant("p109-flow-list-empty")
    expect(await list(api)).toEqual({ data: [], pageCount: 0 })
    expectInvalid(await api.get("/v1/flows?perPage=0"), "perPage")
    expectInvalid(await api.get("/v1/flows?page=0"), "page")
    expectInvalid(await api.get("/v1/flows?startType=sendText"), "startType")
    expectInvalid(await api.get("/v1/flows?folderId=abc"), "folderId")
    expectInvalid(await api.get("/v1/flows?sort[0][id]=active&sort[0][desc]=false"), "sort.0.id")
  })
})

describe("flows.update", () => {
  it("partially updates name / active / enableInInbox (204, no body)", async () => {
    const { api } = await newTenant("p109-flow-update")
    const created = await createFlow(api, { name: "Before" })
    const before = await getFlow(api, created.id)
    const response = await api.patch(`/v1/flows/${created.id}`, { name: "  After  " })
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expect(await getFlow(api, created.id)).toMatchObject({ name: "After", active: true, enableInInbox: true })
    expect((await api.patch(`/v1/flows/${created.id}`, { active: false, enableInInbox: false })).status).toBe(204)
    const after = await getFlow(api, created.id)
    expect(after).toMatchObject({ name: "After", active: false, enableInInbox: false })
    // The graph is not touched.
    expect(after.flowVersions).toEqual(before.flowVersions)
    // An empty body or unchanged values are a no-op 204.
    expect((await api.patch(`/v1/flows/${created.id}`, {})).status).toBe(204)
    expect((await api.patch(`/v1/flows/${created.id}`, { name: "After" })).status).toBe(204)
    expect(await getFlow(api, created.id)).toMatchObject({ name: "After", active: false, enableInInbox: false })
  })

  it("unknown or foreign id -> 404 notFound; bad input -> 422", async () => {
    const { api } = await newTenant("p109-flow-update-bad")
    const { api: otherApi } = await newTenant("p109-flow-update-other")
    const foreign = await createFlow(otherApi, { name: "Foreign" })
    expectError(await api.patch(`/v1/flows/${UNKNOWN_ID}`, { name: "x" }), 404, "notFound", { message: "Flow not found" })
    expectError(await api.patch(`/v1/flows/${foreign.id}`, { name: "x" }), 404, "notFound")
    expect((await getFlow(otherApi, foreign.id)).name).toBe("Foreign")
    const own = await createFlow(api, { name: "Own" })
    expectInvalid(await api.patch(`/v1/flows/${own.id}`, { name: "" }), "name")
    expectInvalid(await api.patch(`/v1/flows/${own.id}`, { active: "yes" }), "active")
    expectInvalid(await api.patch("/v1/flows/abc", { name: "x" }), "id")
  })
})

describe("flows.delete", () => {
  it("deletes the flow and its versions (204); an unknown or foreign id is also 204 and changes nothing", async () => {
    const { api } = await newTenant("p109-flow-delete")
    const { api: otherApi } = await newTenant("p109-flow-delete-other")
    const id = await createPublishedFlow(api, textSpec("Doomed", "x"))
    const keep = await createFlow(api, { name: "Keep" })
    const foreign = await createFlow(otherApi, { name: "Foreign" })
    const response = await api.delete(`/v1/flows/${id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectError(await api.get(`/v1/flows/${id}`), 404, "notFound")
    expectError(await api.get(`/v1/flows/${id}/nodes`), 404, "notFound")
    expect((await list(api)).data.map((flow) => flow.id)).toEqual([keep.id])
    expect((await api.delete(`/v1/flows/${id}`)).status).toBe(204)
    expect((await api.delete(`/v1/flows/${UNKNOWN_ID}`)).status).toBe(204)
    expect((await api.delete(`/v1/flows/${foreign.id}`)).status).toBe(204)
    expect((await getFlow(otherApi, foreign.id)).name).toBe("Foreign")
    expectInvalid(await api.delete("/v1/flows/abc"), "id")
  })
})

describe("flows.duplicate", () => {
  it("copies a published flow as published, keeping active; names _copy, _copy 2", async () => {
    const { api } = await newTenant("p109-flow-dup")
    const name = uniqueName("Orig")
    const id = await createPublishedFlow(api, textSpec(name, "copied text"))
    const source = await getFlow(api, id)
    const response = await api.post<{ id: string }>(`/v1/flows/${id}/duplicate`)
    expect(response.status, response.text).toBe(201)
    expect(Object.keys(response.json)).toEqual(["id"])
    const copy = await getFlow(api, response.json.id)
    expect(copy).toMatchObject({
      name: `${name} _copy`,
      active: true,
      enableInInbox: false,
      folderId: null,
      currentVersionId: expect.stringMatching(/^\d+$/),
    })
    expect(copy.currentVersionId).not.toBe(source.currentVersionId)
    expect(copy.flowVersions).toHaveLength(2)
    const copyDraft = draftOf(copy)
    const copyPublished = copy.flowVersions.find((version) => !version.isDraft)
    expect(copyPublished).toMatchObject({ id: copy.currentVersionId, isLatest: true, flowId: copy.id })
    // The graph is copied as it is (same node ids).
    expect(copyDraft.nodes).toEqual(draftOf(source).nodes)
    expect(copyDraft.startNodeId).toBe(draftOf(source).startNodeId)
    expect(sendTexts(copyPublished as FlowDetail["flowVersions"][number])).toEqual(["copied text"])

    const second = await api.post<{ id: string }>(`/v1/flows/${id}/duplicate`)
    expect((await getFlow(api, second.json.id)).name).toBe(`${name} _copy 2`)
  })

  it("copies a draft-only flow as an inactive draft-only flow", async () => {
    const { api } = await newTenant("p109-flow-dup-draft")
    const created = await createFlow(api, { name: "Draft", spec: textSpec("Draft", "only draft") })
    const response = await api.post<{ id: string }>(`/v1/flows/${created.id}/duplicate`)
    expect(response.status).toBe(201)
    const copy = await getFlow(api, response.json.id)
    expect(copy).toMatchObject({ name: "Draft _copy", active: false, enableInInbox: true, currentVersionId: null })
    expect(copy.flowVersions).toHaveLength(1)
    expect(sendTexts(draftOf(copy))).toEqual(["only draft"])
    expect(nodeSteps(draftOf(copy).nodes[0] ?? {})).toHaveLength(1)
  })

  it("unknown or foreign id -> 404 notFound", async () => {
    const { api } = await newTenant("p109-flow-dup-404")
    const { api: otherApi } = await newTenant("p109-flow-dup-other")
    const foreign = await createFlow(otherApi, { name: "Foreign" })
    expectError(await api.post(`/v1/flows/${UNKNOWN_ID}/duplicate`), 404, "notFound", { message: "Flow not found" })
    expectError(await api.post(`/v1/flows/${foreign.id}/duplicate`), 404, "notFound")
    expect((await list(otherApi)).data).toHaveLength(1)
  })
})
