// miniApps.create / get / list / update / duplicate / delete / deleteMany /
// getFlowJson / validate / listSubmissions. WhatsApp publishing is in
// 51-mini-apps-whatsapp.
//
// A Mini App created from a name alone gets the one-screen starter (a heading
// and a Complete footer). Its Flow JSON is the black-box sample of a valid
// Flow JSON for the other tests. Submissions are written by the public Mini
// App page, which has no public API path: they come from the P1.12 fixture
// extension.

import { describe, expect, it } from "vitest"
import {
  type Api,
  ext,
  expectInvalid,
  expectNotFound,
  expectValidation,
  ISO,
  keysOf,
  newContact,
  newTenant,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueName,
} from "./helpers.ts"

type FlowJson = { version: string; screens: Array<Record<string, unknown> & { id: string }> }

type MiniApp = {
  id: string
  name: string
  enabled: boolean
  submissionsCount: number
  publicUrl: string
  createdAt: string
  updatedAt: string
  flowJson: FlowJson
  customFieldMappings: Record<string, string>
  validation: { valid: boolean; issues: Array<Record<string, unknown>> }
  publications: unknown[]
}

const SUMMARY_KEYS = ["createdAt", "enabled", "id", "name", "publicUrl", "submissionsCount", "updatedAt"]
const DETAIL_KEYS = [...SUMMARY_KEYS, "customFieldMappings", "flowJson", "publications", "validation"].sort()

const create = async (api: Api, body: Record<string, unknown> = {}): Promise<MiniApp> => {
  const response = await api.post<MiniApp>("/v1/mini-apps", { name: uniqueName("Survey"), ...body })
  expect(response.status, response.text).toBe(201)
  return response.json
}

/** A Flow JSON whose only screen has no footer and is not terminal. */
const brokenFlowJson = (): FlowJson => ({
  version: "7.2",
  screens: [
    {
      id: "ONLY",
      title: "Only",
      layout: { type: "SingleColumnLayout", children: [{ type: "TextBody", text: "Hello" }] },
    },
  ],
})

describe("miniApps.create", () => {
  it("201 with the starter screen, a valid definition and a public URL", async () => {
    const { api } = await newTenant("p112-ma-create")
    const name = uniqueName("Survey")
    const response = await api.post<MiniApp>("/v1/mini-apps", { name: `  ${name} ` })
    expect(response.status, response.text).toBe(201)
    const app = response.json
    expect(keysOf(app)).toEqual(DETAIL_KEYS)
    expect(app.id).toMatch(SNOWFLAKE)
    expect(app).toMatchObject({
      name,
      enabled: true,
      submissionsCount: 0,
      publicUrl: `http://app/mini-apps?id=${app.id}&token={{mini_app_token}}`,
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
      customFieldMappings: {},
      validation: { valid: true },
      publications: [],
    })
    expect(app.validation.issues.filter((issue) => issue.severity === "error")).toEqual([])
    expect(typeof app.flowJson.version).toBe("string")
    expect(app.flowJson.screens).toHaveLength(1)
    expect(app.flowJson.screens[0]).toMatchObject({ terminal: true, layout: { type: "SingleColumnLayout" } })
  })

  it("creates from a Flow JSON; 422 for a taken name, an empty name and a Flow JSON without screens", async () => {
    const { api } = await newTenant("p112-ma-create-json")
    const starter = await create(api)
    const copy = await create(api, { flowJson: starter.flowJson })
    expect(copy.flowJson).toEqual(starter.flowJson)
    expect(copy.validation.valid).toBe(true)

    expectValidation(await api.post("/v1/mini-apps", { name: starter.name }), "Name is already taken")
    expectInvalid(await api.post("/v1/mini-apps", { name: "  " }), "name")
    expectInvalid(await api.post("/v1/mini-apps", {}), "name")
    expectInvalid(
      await api.post("/v1/mini-apps", { name: uniqueName("x"), flowJson: { version: "7.2", screens: [] } }),
      "flowJson.screens",
    )
    expectInvalid(await api.post("/v1/mini-apps", { name: uniqueName("x"), flowJson: { screens: [] } }), "flowJson.version")
    const listed = await api.get<{ data: unknown[] }>("/v1/mini-apps")
    expect(listed.json.data).toHaveLength(2)
  })

  it("an invalid Flow JSON is stored with its validation issues", async () => {
    const { api } = await newTenant("p112-ma-create-broken")
    const app = await create(api, { flowJson: brokenFlowJson() })
    expect(app.validation.valid).toBe(false)
    expect(app.validation.issues.length).toBeGreaterThan(0)
    for (const issue of app.validation.issues) {
      expect(typeof issue.code).toBe("string")
      expect(["error", "warning"]).toContain(issue.severity)
    }
  })
})

describe("miniApps.validate", () => {
  it("reports a valid Flow JSON as valid and a broken one with located issues; writes nothing", async () => {
    const { api } = await newTenant("p112-ma-validate")
    const starter = await create(api)
    const valid = await api.post<{ valid: boolean; issues: unknown[] }>("/v1/mini-apps/validate", { flowJson: starter.flowJson })
    expect(valid.status, valid.text).toBe(200)
    expect(keysOf(valid.json)).toEqual(["issues", "valid"])
    expect(valid.json.valid).toBe(true)

    const broken = await api.post<{ valid: boolean; issues: Array<Record<string, unknown>> }>("/v1/mini-apps/validate", {
      flowJson: brokenFlowJson(),
    })
    expect(broken.status, broken.text).toBe(200)
    expect(broken.json.valid).toBe(false)
    expect(broken.json.issues.some((issue) => issue.severity === "error")).toBe(true)
    for (const issue of broken.json.issues) {
      for (const key of Object.keys(issue)) {
        expect(["code", "severity", "screenId", "path", "property", "params"]).toContain(key)
      }
    }
    expectInvalid(await api.post("/v1/mini-apps/validate", {}), "flowJson")
    const listed = await api.get<{ data: unknown[] }>("/v1/mini-apps")
    expect(listed.json.data).toHaveLength(1)
  })
})

describe("miniApps.get / getFlowJson / list", () => {
  it("get and getFlowJson return the stored app; 404 for unknown and foreign ids", async () => {
    const { api } = await newTenant("p112-ma-get")
    const { api: other } = await newTenant("p112-ma-get-other")
    const app = await create(api)
    const response = await api.get<MiniApp>(`/v1/mini-apps/${app.id}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual(app)
    const flowJson = await api.get<FlowJson>(`/v1/mini-apps/${app.id}/flow-json`)
    expect(flowJson.status, flowJson.text).toBe(200)
    expect(flowJson.json).toEqual(app.flowJson)
    expectNotFound(await other.get(`/v1/mini-apps/${app.id}`), "Mini App not found")
    expectNotFound(await other.get(`/v1/mini-apps/${app.id}/flow-json`), "Mini App not found")
    expectNotFound(await api.get(`/v1/mini-apps/${UNKNOWN_ID}`), "Mini App not found")
    expectInvalid(await api.get("/v1/mini-apps/abc"), "id")
  })

  it("lists summaries newest first with pages and a name filter", async () => {
    const { api } = await newTenant("p112-ma-list")
    const token = uniqueName("lst")
    const apps: MiniApp[] = []
    for (let index = 0; index < 3; index += 1) {
      apps.push(await create(api, { name: `${token} app ${index}` }))
    }
    const solo = await create(api)
    const page1 = await api.get<{ data: Array<Record<string, unknown>>; pageCount: number }>("/v1/mini-apps", {
      query: { perPage: 3 },
    })
    expect(page1.status, page1.text).toBe(200)
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data.map((app) => app.id)).toEqual([solo.id, apps[2]?.id, apps[1]?.id])
    expect(keysOf(page1.json.data[0])).toEqual(SUMMARY_KEYS)
    expect(page1.json.data[0]).toEqual({
      id: solo.id,
      name: solo.name,
      enabled: true,
      submissionsCount: 0,
      publicUrl: solo.publicUrl,
      createdAt: solo.createdAt,
      updatedAt: solo.updatedAt,
    })
    const page2 = await api.get<{ data: Array<{ id: string }> }>("/v1/mini-apps", { query: { perPage: 3, page: 2 } })
    expect(page2.json.data.map((app) => app.id)).toEqual([apps[0]?.id])
    const filtered = await api.get<{ data: Array<{ id: string }> }>("/v1/mini-apps", { query: { name: token.toUpperCase() } })
    expect(filtered.json.data.map((app) => app.id)).toEqual([apps[2]?.id, apps[1]?.id, apps[0]?.id])
    expectInvalid(await api.get("/v1/mini-apps", { query: { perPage: 51 } }), "perPage")
  })
})

describe("miniApps.update", () => {
  it("renames, disables and replaces the screens from a Flow JSON", async () => {
    const { api } = await newTenant("p112-ma-update")
    const app = await create(api)
    const renamed = await api.patch<MiniApp>(`/v1/mini-apps/${app.id}`, { name: "Renamed app", enabled: false })
    expect(renamed.status, renamed.text).toBe(200)
    expect(renamed.json).toMatchObject({ id: app.id, name: "Renamed app", enabled: false, flowJson: app.flowJson })

    const replaced = await api.patch<MiniApp>(`/v1/mini-apps/${app.id}`, { flowJson: brokenFlowJson() })
    expect(replaced.status, replaced.text).toBe(200)
    expect(replaced.json.validation.valid).toBe(false)
    expect(replaced.json.flowJson.screens.map((screen) => screen.id)).toEqual(["ONLY"])
    expect(replaced.json).toMatchObject({ name: "Renamed app", enabled: false })

    const unchanged = await api.patch<MiniApp>(`/v1/mini-apps/${app.id}`, {})
    expect(unchanged.status, unchanged.text).toBe(200)
    expect(unchanged.json).toEqual(replaced.json)
    expect((await api.get<MiniApp>(`/v1/mini-apps/${app.id}`)).json).toEqual(replaced.json)
  })

  it("422 for a taken name; 404 for unknown and foreign ids", async () => {
    const { api } = await newTenant("p112-ma-update-err")
    const { api: other } = await newTenant("p112-ma-update-err-other")
    const app = await create(api)
    const taken = await create(api)
    const foreign = await create(other)
    expectValidation(await api.patch(`/v1/mini-apps/${app.id}`, { name: taken.name }), "Name is already taken")
    expectNotFound(await api.patch(`/v1/mini-apps/${foreign.id}`, { name: "x" }), "Mini App not found")
    expectNotFound(await api.patch(`/v1/mini-apps/${UNKNOWN_ID}`, { name: "x" }), "Mini App not found")
    expect((await other.get<MiniApp>(`/v1/mini-apps/${foreign.id}`)).json.name).toBe(foreign.name)
  })
})

describe("miniApps.duplicate", () => {
  it("copies the screens as '<name> - Copy', then '- Copy 2'; submissions stay with the original", async () => {
    const { api, tenant } = await newTenant("p112-ma-duplicate")
    const app = await create(api)
    await (await ext()).createMiniAppSubmission({ workspaceId: tenant.workspaceId, miniAppId: app.id, answers: { a: "1" } })
    const first = await api.post<MiniApp>(`/v1/mini-apps/${app.id}/duplicate`, {})
    expect(first.status, first.text).toBe(201)
    expect(keysOf(first.json)).toEqual(DETAIL_KEYS)
    expect(first.json).toMatchObject({
      name: `${app.name} - Copy`,
      enabled: true,
      submissionsCount: 0,
      flowJson: app.flowJson,
      publications: [],
    })
    expect(first.json.id).not.toBe(app.id)
    const second = await api.post<MiniApp>(`/v1/mini-apps/${app.id}/duplicate`, {})
    expect(second.json.name).toBe(`${app.name} - Copy 2`)

    const disabled = await create(api)
    expect((await api.patch(`/v1/mini-apps/${disabled.id}`, { enabled: false })).status).toBe(200)
    const disabledCopy = await api.post<MiniApp>(`/v1/mini-apps/${disabled.id}/duplicate`, {})
    expect(disabledCopy.json.enabled).toBe(false)
    expectNotFound(await api.post(`/v1/mini-apps/${UNKNOWN_ID}/duplicate`, {}), "Mini App not found")
  })
})

describe("miniApps.delete / deleteMany", () => {
  it("delete returns the count; 404 for unknown and foreign ids", async () => {
    const { api } = await newTenant("p112-ma-delete")
    const { api: other } = await newTenant("p112-ma-delete-other")
    const app = await create(api)
    const foreign = await create(other)
    const response = await api.delete<{ deletedCount: number; whatsappFlows: unknown[] }>(`/v1/mini-apps/${app.id}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ deletedCount: 1, whatsappFlows: [] })
    expectNotFound(await api.get(`/v1/mini-apps/${app.id}`), "Mini App not found")
    expectNotFound(await api.delete(`/v1/mini-apps/${app.id}`), "Mini App not found")
    expectNotFound(await api.delete(`/v1/mini-apps/${foreign.id}`), "Mini App not found")
    expect((await other.get(`/v1/mini-apps/${foreign.id}`)).status).toBe(200)
  })

  it("deleteMany deletes only this workspace's apps and counts them", async () => {
    const { api } = await newTenant("p112-ma-bulk")
    const { api: other } = await newTenant("p112-ma-bulk-other")
    const first = await create(api)
    const second = await create(api)
    const kept = await create(api)
    const foreign = await create(other)
    const response = await api.post<{ deletedCount: number; whatsappFlows: unknown[] }>("/v1/mini-apps/bulk-delete", {
      ids: [first.id, second.id, foreign.id, UNKNOWN_ID],
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ deletedCount: 2, whatsappFlows: [] })
    const listed = await api.get<{ data: Array<{ id: string }> }>("/v1/mini-apps")
    expect(listed.json.data.map((app) => app.id)).toEqual([kept.id])
    expect((await other.get(`/v1/mini-apps/${foreign.id}`)).status).toBe(200)
    expectInvalid(await api.post("/v1/mini-apps/bulk-delete", { ids: [] }), "ids")
    expectInvalid(
      await api.post("/v1/mini-apps/bulk-delete", { ids: Array.from({ length: 101 }, () => UNKNOWN_ID) }),
      "ids",
    )
  })
})

describe("miniApps.listSubmissions", () => {
  it("lists the answers newest first with pages; the count follows", async () => {
    const { api, tenant } = await newTenant("p112-ma-subs")
    const app = await create(api)
    const contactId = await newContact(api)
    const fixtures = await ext()
    const now = Date.now()
    const written = []
    for (let index = 0; index < 3; index += 1) {
      written.push(
        await fixtures.createMiniAppSubmission({
          workspaceId: tenant.workspaceId,
          miniAppId: app.id,
          contactId: index === 1 ? contactId : null,
          answers: { favourite: `answer ${index}`, score: index },
          createdAt: new Date(now - (3 - index) * 1_000).toISOString(),
        }),
      )
    }
    const page1 = await api.get<{ data: unknown[]; pageCount: number }>(`/v1/mini-apps/${app.id}/submissions`, {
      query: { perPage: 2 },
    })
    expect(page1.status, page1.text).toBe(200)
    expect(page1.json).toEqual({
      data: [
        { id: written[2]?.submissionId, contactId: null, answers: { favourite: "answer 2", score: 2 }, createdAt: written[2]?.createdAt },
        { id: written[1]?.submissionId, contactId, answers: { favourite: "answer 1", score: 1 }, createdAt: written[1]?.createdAt },
      ],
      pageCount: 2,
    })
    const page2 = await api.get<{ data: Array<{ id: string }> }>(`/v1/mini-apps/${app.id}/submissions`, {
      query: { perPage: 2, page: 2 },
    })
    expect(page2.json.data.map((submission) => submission.id)).toEqual([written[0]?.submissionId])
    expect((await api.get<MiniApp>(`/v1/mini-apps/${app.id}`)).json.submissionsCount).toBe(3)
  })

  it("empty for a new app; 404 for unknown and foreign ids", async () => {
    const { api } = await newTenant("p112-ma-subs-404")
    const { api: other } = await newTenant("p112-ma-subs-404-other")
    const app = await create(api)
    const empty = await api.get(`/v1/mini-apps/${app.id}/submissions`)
    expect(empty.json).toEqual({ data: [], pageCount: 0 })
    expectNotFound(await other.get(`/v1/mini-apps/${app.id}/submissions`), "Mini App not found")
    expectNotFound(await api.get(`/v1/mini-apps/${UNKNOWN_ID}/submissions`), "Mini App not found")
  })
})
