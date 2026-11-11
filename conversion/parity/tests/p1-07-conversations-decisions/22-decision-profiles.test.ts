// decisionProfiles.create / get / list / update / setEnabled / deleteMany /
// listGlobalRouterDestinations (scope `automation`). A profile references a
// decision connection (created through decisionConnections.create; no
// provider call) and, with Global Router on, flows / AI agents / users /
// inbox teams / custom fields.

import { describe, expect, it } from "vitest"
import { fixtures } from "../../src/fixtures/index.ts"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueEmail } from "../../src/unique.ts"
import {
  choiceDecision,
  createCompatibleConnection,
  createField,
  createProfile,
  expectInvalidAt,
  filledRouterConfig,
  type Profile,
  profileInput,
  routerConfig,
} from "./helpers-decisions.ts"
import { type Api, createPublishedFlow, expectError, ext, newTenant, sortBy, UNKNOWN_ID, uniqueName } from "./helpers.ts"

type ProfileList = { data: Profile[]; pageCount: number }
type Destinations = {
  agents: Array<{ id: string; name: string }>
  customFields: Array<{ id: string; name: string; type: string }>
  flows: Array<{ id: string; name: string }>
  teams: Array<{ id: string; name: string }>
  users: Array<{ id: string; name: string }>
}

const PATH = "/v1/decision-profiles"

/** Tenant + an enabled System One Compatible connection with one model. */
const setup = async (label: string) => {
  const { tenant, api } = await newTenant(label)
  const { connection, model } = await createCompatibleConnection(api)
  return { tenant, api, connectionId: connection.id, model }
}

const createDraftFlow = async (api: Api, name: string): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/flows", { name, folderId: null })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

const createAiAgent = async (api: Api, name: string): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/ai-agents", {
    name,
    prompt: "You route parity tests.",
    messages: [],
    models: [{ provider: "openai", model: "gpt-4o-mini" }],
    temperature: 0.2,
    maxOutputTokens: 200,
    tools: [],
    isDefault: false,
  })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

describe("decisionProfiles.create", () => {
  it("creates a choice profile (201) in the authoring shape", async () => {
    const n = new Normalizer()
    const { api, connectionId, model } = await setup("p107-dp-create")
    const name = uniqueName("Intent")
    const created = await api.post<Profile>(PATH, {
      name: `  ${name}  `,
      connectionId,
      model: ` ${model} `,
      description: "  Routes by intent  ",
      decision: {
        type: "choice",
        instructions: "  Pick the intent.  ",
        options: [
          { value: "buy", description: "Wants to buy." },
          { value: "support_request", description: "Needs help." },
        ],
      },
      thresholdConfig: { type: "choice_confidence", minimum: 0.6 },
    })
    expect(created.status, created.text).toBe(201)
    expect(n.normalize(created.json, { replace: { [connectionId]: "<connection>" } })).toEqual({
      id: "<id:1>",
      connectionId: "<connection>",
      name,
      model,
      description: "Routes by intent",
      status: "enabled",
      decision: {
        type: "choice",
        instructions: "Pick the intent.",
        options: [
          { value: "buy", description: "Wants to buy." },
          { value: "support_request", description: "Needs help." },
        ],
      },
      thresholdConfig: { type: "choice_confidence", minimum: 0.6 },
      globalRouterEnabled: false,
      globalRouterConfig: null,
    })
    const got = await api.get<Profile>(`${PATH}/${created.json.id}`)
    expect(got.status, got.text).toBe(200)
    expect(got.json).toEqual(created.json)
  })

  it("score and noul profiles keep their own decision shape; blank description is null", async () => {
    const { api, connectionId, model } = await setup("p107-dp-kinds")
    const score = await createProfile(api, connectionId, model, {
      description: "   ",
      status: "disabled",
      decision: {
        type: "score",
        instructions: "Rate the urgency.",
        levels: [{ description: "Low" }, { description: "Medium" }, { description: "High" }],
      },
      thresholdConfig: { type: "score", operator: "gte", value: 2 },
    })
    expect(score).toMatchObject({
      description: null,
      status: "disabled",
      decision: {
        type: "score",
        instructions: "Rate the urgency.",
        levels: [{ description: "Low" }, { description: "Medium" }, { description: "High" }],
      },
      thresholdConfig: { type: "score", operator: "gte", value: 2 },
    })
    const noul = await createProfile(api, connectionId, model, {
      decision: {
        type: "noul",
        instructions: "Is this spam?",
        trueCriteria: "Unsolicited advertising.",
        falseCriteria: "A real question.",
      },
      thresholdConfig: { type: "noul_true_probability", minimum: 0.7 },
    })
    expect(noul).toMatchObject({
      description: null,
      decision: {
        type: "noul",
        instructions: "Is this spam?",
        trueCriteria: "Unsolicited advertising.",
        falseCriteria: "A real question.",
      },
      thresholdConfig: { type: "noul_true_probability", minimum: 0.7 },
    })
  })

  it("validates the input (422 invalidRequestData)", async () => {
    const { api, connectionId, model } = await setup("p107-dp-invalid")
    const ok = profileInput(connectionId, model)
    const bad = (overrides: Record<string, unknown>) => api.post(PATH, { ...ok, ...overrides })
    expectInvalidAt(await bad({ name: " " }), "name")
    expectInvalidAt(await bad({ model: "" }), "model")
    expectInvalidAt(await bad({ connectionId: "abc" }), "connectionId")
    expectInvalidAt(
      await bad({ decision: { ...choiceDecision, options: [{ value: "only", description: "One" }] } }),
      "decision.options",
    )
    expectInvalidAt(
      await bad({
        decision: {
          ...choiceDecision,
          options: [
            { value: "same", description: "A" },
            { value: "same", description: "B" },
          ],
        },
      }),
      "decision.options",
    )
    expectInvalidAt(
      await bad({
        decision: {
          ...choiceDecision,
          options: [
            { value: "Buy", description: "A" },
            { value: "sell", description: "B" },
          ],
        },
      }),
      "decision.options.0.value",
    )
    expectInvalidAt(await bad({ decision: { type: "score", instructions: "x", levels: [{ description: "a" }] } }), "decision.levels")
    expectInvalidAt(
      await bad({
        decision: { type: "noul", instructions: "i".repeat(500), trueCriteria: "t".repeat(300), falseCriteria: "f".repeat(300) },
      }),
      "decision",
    )
    expectInvalidAt(await bad({ thresholdConfig: { type: "score", operator: "gte", value: 1 } }), "thresholdConfig")
    expectInvalidAt(
      await bad({
        decision: { type: "score", instructions: "x", levels: [{ description: "a" }, { description: "b" }] },
        thresholdConfig: { type: "score", operator: "gte", value: 3 },
      }),
      "thresholdConfig",
    )
    expectInvalidAt(await bad({ globalRouterEnabled: true }), "globalRouterConfig")
    expectError(await bad({ contract: {} }), 422, "invalidRequestData")
    expectError(await bad({ decision: { type: "vote" } }), 422, "invalidRequestData")
    expect((await api.get<ProfileList>(PATH)).json.data).toEqual([])
  })

  it("checks the connection: unknown / foreign 404, model outside the catalog 400; a disabled one is accepted", async () => {
    const { api, connectionId, model } = await setup("p107-dp-connection")
    const other = await setup("p107-dp-connection-other")
    // `notFound` is not in the create route's error map, so it is not `defined`.
    expectError(await api.post(PATH, profileInput(UNKNOWN_ID, model)), 404, "notFound", {
      defined: false,
      message: "Decision connection not found",
    })
    expectError(await api.post(PATH, profileInput(other.connectionId, other.model)), 404, "notFound", {
      message: "Decision connection not found",
    })
    expectError(await api.post(PATH, profileInput(connectionId, "not-in-catalog")), 400, "invalidDecisionModel", {
      defined: true,
      message: "Decision model is not available on this connection",
    })
    expect((await api.patch(`/v1/decision-connections/${connectionId}/status`, { enabled: false })).status).toBe(200)
    const onDisabled = await api.post<Profile>(PATH, profileInput(connectionId, model))
    expect(onDisabled.status, onDisabled.text).toBe(201)
    expect(onDisabled.json).toMatchObject({ connectionId, status: "enabled" })
  })

  it("a second profile with the same name is refused and not stored", async () => {
    const { api, connectionId, model } = await setup("p107-dp-dup")
    const first = await createProfile(api, connectionId, model)
    const again = await api.post(PATH, profileInput(connectionId, model, { name: first.name }))
    expect(again.status, again.text).toBe(500)
    expect((again.json as { code?: string }).code).toBe("INTERNAL_SERVER_ERROR")
    expect((await api.get<ProfileList>(PATH)).json.data.map((profile) => profile.id)).toEqual([first.id])
  })
})

describe("decisionProfiles Global Router", () => {
  it("enables the router with a published-flow fallback; config defaults are filled in", async () => {
    const { api, connectionId, model } = await setup("p107-dp-router")
    const flowId = await createPublishedFlow(api, uniqueName("Fallback"))
    const created = await api.post<Profile>(
      PATH,
      profileInput(connectionId, model, {
        globalRouterEnabled: true,
        globalRouterConfig: routerConfig(
          { type: "flow", id: flowId },
          { routes: [{ type: "choice", value: "buy", destination: { type: "flow", id: flowId } }] },
        ),
      }),
    )
    expect(created.status, created.text).toBe(201)
    expect(created.json.globalRouterEnabled).toBe(true)
    expect(created.json.globalRouterConfig).toEqual(
      filledRouterConfig(
        { type: "flow", id: flowId },
        { routes: [{ type: "choice", value: "buy", destination: { type: "flow", id: flowId } }] },
      ),
    )
    expect((await api.get<Profile>(`${PATH}/${created.json.id}`)).json).toEqual(created.json)
  })

  it("only one router per workspace: a new router turns the previous one off", async () => {
    const { tenant, api, connectionId, model } = await setup("p107-dp-router-one")
    const fallback = { type: "user", id: tenant.userId }
    const first = await createProfile(api, connectionId, model, {
      globalRouterEnabled: true,
      globalRouterConfig: routerConfig(fallback),
    })
    expect(first.globalRouterEnabled).toBe(true)
    const second = await createProfile(api, connectionId, model, {
      globalRouterEnabled: true,
      globalRouterConfig: routerConfig(fallback),
    })
    expect(second.globalRouterEnabled).toBe(true)
    expect((await api.get<Profile>(`${PATH}/${first.id}`)).json.globalRouterEnabled).toBe(false)
    // The previous router keeps its config.
    expect((await api.get<Profile>(`${PATH}/${first.id}`)).json.globalRouterConfig).toEqual(
      filledRouterConfig(fallback),
    )
    // Turning the first back on through update turns the second off.
    const { id: _id, ...firstBody } = first
    const back = await api.put<Profile>(`${PATH}/${first.id}`, { ...firstBody, globalRouterEnabled: true })
    expect(back.status, back.text).toBe(200)
    expect(back.json.globalRouterEnabled).toBe(true)
    expect((await api.get<Profile>(`${PATH}/${second.id}`)).json.globalRouterEnabled).toBe(false)
  })

  it("accepts AI agent, user and inbox-team destinations and metadata mappings to custom fields", async () => {
    const { tenant, api, connectionId, model } = await setup("p107-dp-router-dest")
    const fx = await ext()
    const { inboxTeamId } = await fx.createInboxTeam({ workspaceId: tenant.workspaceId, name: uniqueName("Team") })
    const agentId = await createAiAgent(api, uniqueName("Agent"))
    const choiceField = await createField(api, { name: uniqueName("Choice"), type: "shortText" })
    const confidenceField = await createField(api, { name: uniqueName("Confidence"), type: "number" })
    const config = {
      fallback: { type: "team", id: inboxTeamId },
      routes: [
        { type: "choice", value: "buy", destination: { type: "aiAgent", id: agentId } },
        { type: "choice", value: "support", destination: { type: "user", id: tenant.userId } },
      ],
      metadataMappings: [
        { value: "choice", customFieldId: choiceField.id, customFieldType: "shortText" },
        { value: "confidence", customFieldId: confidenceField.id, customFieldType: "number" },
      ],
      noul: { falseMaximum: 0.3, trueMinimum: 0.6 },
    }
    const created = await api.post<Profile>(
      PATH,
      profileInput(connectionId, model, { globalRouterEnabled: true, globalRouterConfig: config }),
    )
    expect(created.status, created.text).toBe(201)
    expect(created.json.globalRouterConfig).toEqual({ ...config, version: 1 })
  })

  it("refuses unavailable references and mismatches (400 invalidDecisionReference)", async () => {
    const { tenant, api, connectionId, model } = await setup("p107-dp-router-bad")
    const draftFlowId = await createDraftFlow(api, uniqueName("Draft"))
    const emailField = await createField(api, { name: uniqueName("Email"), type: "email" })
    const numberField = await createField(api, { name: uniqueName("Num"), type: "number" })
    const userFallback = { type: "user", id: tenant.userId }
    const cases: Array<[Record<string, unknown>, string]> = [
      [routerConfig({ type: "flow", id: draftFlowId }), "Global Router Flow must be published"],
      [routerConfig({ type: "flow", id: UNKNOWN_ID }), "Global Router Flow must be published"],
      [routerConfig({ type: "aiAgent", id: UNKNOWN_ID }), "Global Router AI Agent is unavailable"],
      [routerConfig({ type: "user", id: UNKNOWN_ID }), "Global Router user is unavailable"],
      [routerConfig({ type: "team", id: UNKNOWN_ID }), "Global Router team is unavailable"],
      [
        routerConfig(userFallback, {
          routes: [{ type: "choice", value: "refund", destination: userFallback }],
        }),
        "Global Router choice route is not in the Decision options",
      ],
      [
        routerConfig(userFallback, {
          routes: [{ type: "score", from: 1, to: 2, destination: userFallback }],
        }),
        "Global Router routes must match the Decision question type",
      ],
      [
        routerConfig(userFallback, {
          metadataMappings: [{ value: "choice", customFieldId: UNKNOWN_ID, customFieldType: "shortText" }],
        }),
        "Global Router metadata field is unavailable",
      ],
      [
        routerConfig(userFallback, {
          metadataMappings: [{ value: "choice", customFieldId: emailField.id, customFieldType: "shortText" }],
        }),
        "Global Router metadata mapping is incompatible",
      ],
      [
        routerConfig(userFallback, {
          metadataMappings: [{ value: "score", customFieldId: numberField.id, customFieldType: "number" }],
        }),
        "Global Router metadata mapping is incompatible",
      ],
    ]
    for (const [globalRouterConfig, message] of cases) {
      const response = await api.post(
        PATH,
        profileInput(connectionId, model, { globalRouterEnabled: true, globalRouterConfig }),
      )
      expectError(response, 400, "invalidDecisionReference", { defined: true, message })
    }
    const disabled = await api.post(
      PATH,
      profileInput(connectionId, model, {
        status: "disabled",
        globalRouterEnabled: true,
        globalRouterConfig: routerConfig(userFallback),
      }),
    )
    expectError(disabled, 400, "invalidDecisionReference", { message: "Global Router profile must be enabled" })
    expect((await api.get<ProfileList>(PATH)).json.data).toEqual([])
  })

  it("schema rules of the router config (422)", async () => {
    const { tenant, api, connectionId, model } = await setup("p107-dp-router-schema")
    const user = { type: "user", id: tenant.userId }
    const post = (globalRouterConfig: unknown) =>
      api.post(PATH, profileInput(connectionId, model, { globalRouterEnabled: true, globalRouterConfig }))
    expectInvalidAt(await post({ fallback: user, noul: { falseMaximum: 0.8, trueMinimum: 0.2 } }), "globalRouterConfig.noul.falseMaximum")
    expectInvalidAt(
      await post({
        fallback: user,
        routes: [
          { type: "choice", value: "buy", destination: user },
          { type: "choice", value: "buy", destination: user },
        ],
      }),
      "globalRouterConfig.routes.1",
    )
    expectInvalidAt(await post({ fallback: { type: "user", id: "me" } }), "globalRouterConfig.fallback.id")
    expectInvalidAt(await post({ fallback: user, version: 2 }), "globalRouterConfig.version")
    expectError(await post({}), 422, "invalidRequestData")
  })

  it("does not check references while the router is off, nor the model while it is on", async () => {
    const { api, connectionId, model } = await setup("p107-dp-router-lax")
    const off = await api.post<Profile>(
      PATH,
      profileInput(connectionId, model, { globalRouterConfig: routerConfig({ type: "flow", id: UNKNOWN_ID }) }),
    )
    expect(off.status, off.text).toBe(201)
    expect(off.json).toMatchObject({
      globalRouterEnabled: false,
      globalRouterConfig: filledRouterConfig({ type: "flow", id: UNKNOWN_ID }),
    })
    const flowId = await createPublishedFlow(api, uniqueName("Flow"))
    const offCatalog = await api.post<Profile>(
      PATH,
      profileInput(connectionId, "not-in-catalog", {
        globalRouterEnabled: true,
        globalRouterConfig: routerConfig({ type: "flow", id: flowId }),
      }),
    )
    expect(offCatalog.status, offCatalog.text).toBe(201)
    expect(offCatalog.json).toMatchObject({ model: "not-in-catalog", globalRouterEnabled: true })
  })
})

describe("decisionProfiles.get / list", () => {
  it("get is 404 for an unknown or foreign id and 422 for a non-numeric id", async () => {
    const { api, connectionId, model } = await setup("p107-dp-get")
    const other = await newTenant("p107-dp-get-other")
    const profile = await createProfile(api, connectionId, model)
    expectError(await api.get(`${PATH}/${UNKNOWN_ID}`), 404, "notFound", {
      defined: true,
      message: "Decision profile not found",
    })
    expectError(await other.api.get(`${PATH}/${profile.id}`), 404, "notFound")
    expectInvalidAt(await api.get(`${PATH}/abc`), "id")
  })

  it("lists by name ascending, paged; pageCount is 0 when empty", async () => {
    const { api, connectionId, model } = await setup("p107-dp-list")
    expect((await api.get<ProfileList>(PATH)).json).toEqual({ data: [], pageCount: 0 })
    const charlie = await createProfile(api, connectionId, model, { name: "Charlie" })
    const alpha = await createProfile(api, connectionId, model, { name: "Alpha" })
    const bravo = await createProfile(api, connectionId, model, { name: "Bravo" })
    const all = await api.get<ProfileList>(PATH)
    expect(all.status, all.text).toBe(200)
    expect(all.json).toEqual({ data: [alpha, bravo, charlie], pageCount: 1 })
    const page1 = await api.get<ProfileList>(PATH, { query: { page: 1, perPage: 2 } })
    expect(page1.json).toEqual({ data: [alpha, bravo], pageCount: 2 })
    const page2 = await api.get<ProfileList>(PATH, { query: { page: 2, perPage: 2 } })
    expect(page2.json).toEqual({ data: [charlie], pageCount: 2 })
    const other = await newTenant("p107-dp-list-other")
    expect((await other.api.get<ProfileList>(PATH)).json).toEqual({ data: [], pageCount: 0 })
    expectInvalidAt(await api.get(PATH, { query: { perPage: 51 } }), "perPage")
    expectInvalidAt(await api.get(PATH, { query: { page: 0 } }), "page")
  })
})

describe("decisionProfiles.update", () => {
  it("replaces the whole configuration (200); omitted defaults reset", async () => {
    const { api, connectionId, model } = await setup("p107-dp-update")
    const profile = await createProfile(api, connectionId, model, {
      description: "first",
      thresholdConfig: { type: "choice_confidence", minimum: 0.5 },
    })
    const newName = uniqueName("Renamed")
    const updated = await api.put<Profile>(`${PATH}/${profile.id}`, {
      name: newName,
      connectionId,
      model,
      decision: {
        type: "noul",
        instructions: "Is it urgent?",
        trueCriteria: "Deadline today.",
        falseCriteria: "No deadline.",
      },
    })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).toEqual({
      id: profile.id,
      connectionId,
      model,
      name: newName,
      description: null,
      status: "enabled",
      decision: {
        type: "noul",
        instructions: "Is it urgent?",
        trueCriteria: "Deadline today.",
        falseCriteria: "No deadline.",
      },
      thresholdConfig: null,
      globalRouterEnabled: false,
      globalRouterConfig: null,
    })
    expect((await api.get<Profile>(`${PATH}/${profile.id}`)).json).toEqual(updated.json)
  })

  it("moves the profile to another connection; checks connection, model and id", async () => {
    const { api, connectionId, model } = await setup("p107-dp-update-conn")
    const other = await setup("p107-dp-update-conn-other")
    const second = await createCompatibleConnection(api)
    const profile = await createProfile(api, connectionId, model)
    const { id: _id, ...body } = profile
    const moved = await api.put<Profile>(`${PATH}/${profile.id}`, {
      ...body,
      connectionId: second.connection.id,
      model: second.model,
    })
    expect(moved.status, moved.text).toBe(200)
    expect(moved.json).toMatchObject({ connectionId: second.connection.id, model: second.model })
    // The old body (first connection, its model) is valid again.
    expect((await api.put(`${PATH}/${profile.id}`, body)).status).toBe(200)
    expectError(await api.put(`${PATH}/${profile.id}`, { ...body, model: "nope" }), 400, "invalidDecisionModel", {
      message: "Decision model is not available on this connection",
    })
    expectError(
      await api.put(`${PATH}/${profile.id}`, { ...body, connectionId: other.connectionId, model: other.model }),
      404,
      "notFound",
      { message: "Decision connection not found" },
    )
    expectError(await api.put(`${PATH}/${UNKNOWN_ID}`, body), 404, "notFound", {
      defined: true,
      message: "Decision profile not found",
    })
    expectError(await other.api.put(`${PATH}/${profile.id}`, { ...body, connectionId: other.connectionId, model: other.model }), 404, "notFound", {
      message: "Decision profile not found",
    })
    expectInvalidAt(await api.put(`${PATH}/${profile.id}`, { ...body, name: "" }), "name")
    expectInvalidAt(await api.put(`${PATH}/abc`, body), "id")
    expect((await api.get<Profile>(`${PATH}/${profile.id}`)).json).toEqual(profile)
  })
})

describe("decisionProfiles.setEnabled", () => {
  it("disables and re-enables (200, profile shape); 404 unknown / foreign; strict body", async () => {
    const { api, connectionId, model } = await setup("p107-dp-status")
    const other = await newTenant("p107-dp-status-other")
    const profile = await createProfile(api, connectionId, model)
    const disabled = await api.patch<Profile>(`${PATH}/${profile.id}/status`, { enabled: false })
    expect(disabled.status, disabled.text).toBe(200)
    expect(disabled.json).toEqual({ ...profile, status: "disabled" })
    expect((await api.get<Profile>(`${PATH}/${profile.id}`)).json.status).toBe("disabled")
    const enabled = await api.patch<Profile>(`${PATH}/${profile.id}/status`, { enabled: true })
    expect(enabled.json).toEqual(profile)
    expectError(await api.patch(`${PATH}/${UNKNOWN_ID}/status`, { enabled: true }), 404, "notFound", {
      defined: true,
      message: "Decision profile not found",
    })
    expectError(await other.api.patch(`${PATH}/${profile.id}/status`, { enabled: false }), 404, "notFound")
    expectInvalidAt(await api.patch(`${PATH}/${profile.id}/status`, {}), "enabled")
    expectError(await api.patch(`${PATH}/${profile.id}/status`, { enabled: false, x: 1 }), 422, "invalidRequestData")
    expect((await api.get<Profile>(`${PATH}/${profile.id}`)).json.status).toBe("enabled")
  })

  it("disabling the Global Router profile keeps globalRouterEnabled", async () => {
    const { tenant, api, connectionId, model } = await setup("p107-dp-status-router")
    const router = await createProfile(api, connectionId, model, {
      globalRouterEnabled: true,
      globalRouterConfig: routerConfig({ type: "user", id: tenant.userId }),
    })
    const disabled = await api.patch<Profile>(`${PATH}/${router.id}/status`, { enabled: false })
    expect(disabled.json).toMatchObject({ status: "disabled", globalRouterEnabled: true })
  })
})

describe("decisionProfiles.deleteMany", () => {
  it("deletes the given ids (204); unknown and foreign ids are ignored", async () => {
    const { api, connectionId, model } = await setup("p107-dp-delete")
    const other = await setup("p107-dp-delete-other")
    const keep = await createProfile(api, connectionId, model, { name: "Keep" })
    const a = await createProfile(api, connectionId, model, { name: "A" })
    const b = await createProfile(api, connectionId, model, { name: "B" })
    const foreign = await createProfile(other.api, other.connectionId, other.model)
    const deleted = await api.delete(PATH, { json: { ids: [a.id, b.id, a.id, UNKNOWN_ID, foreign.id] } })
    expect(deleted.status, deleted.text).toBe(204)
    expect(deleted.text).toBe("")
    expect((await api.get<ProfileList>(PATH)).json.data).toEqual([keep])
    expectError(await api.get(`${PATH}/${a.id}`), 404, "notFound")
    expect((await other.api.get<Profile>(`${PATH}/${foreign.id}`)).json).toEqual(foreign)
    expect((await api.delete(PATH, { json: { ids: [a.id] } })).status).toBe(204)
  })

  it("validates the body (422)", async () => {
    const { api } = await newTenant("p107-dp-delete-invalid")
    expectInvalidAt(await api.delete(PATH, { json: { ids: [] } }), "ids")
    expectInvalidAt(await api.delete(PATH, { json: {} }), "ids")
    expectInvalidAt(await api.delete(PATH, { json: { ids: ["abc"] } }), "ids.0")
    expectError(await api.delete(PATH, { json: { ids: [UNKNOWN_ID], all: true } }), 422, "invalidRequestData")
  })
})

describe("decisionProfiles.listGlobalRouterDestinations", () => {
  it("a new workspace offers only its owner", async () => {
    const { tenant, api } = await newTenant("p107-dp-dest-empty")
    const response = await api.get<Destinations>(`${PATH}/global-router-destinations`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      agents: [],
      customFields: [],
      flows: [],
      teams: [],
      users: [{ id: tenant.userId, name: "Parity p107-dp-dest-empty" }],
    })
  })

  it("lists published flows, AI agents, text/number custom fields, members and inbox teams", async () => {
    const { tenant, api } = await newTenant("p107-dp-dest")
    const fx = await ext()
    const driver = await fixtures()
    // Teams are cached per workspace on first read: create them first.
    const teamName = uniqueName("Team")
    const { inboxTeamId } = await fx.createInboxTeam({ workspaceId: tenant.workspaceId, name: teamName })
    const agentEmail = uniqueEmail("agent")
    const { userId: agentUserId } = await driver.createUser({
      email: agentEmail,
      password: "Parity#12345",
      name: "Parity Agent Member",
    })
    await fx.addMember({ workspaceId: tenant.workspaceId, userId: agentUserId })

    const flowName = uniqueName("Published")
    const flowId = await createPublishedFlow(api, flowName)
    await createDraftFlow(api, uniqueName("Draft"))
    const agentName = uniqueName("Agent")
    const agentId = await createAiAgent(api, agentName)
    const fields = []
    for (const type of ["shortText", "longText", "number", "email", "boolean", "date"]) {
      fields.push(await createField(api, { name: uniqueName(type), type }))
    }
    const response = await api.get<Destinations>(`${PATH}/global-router-destinations`)
    expect(response.status, response.text).toBe(200)
    const body = response.json
    expect(Object.keys(body).sort()).toEqual(["agents", "customFields", "flows", "teams", "users"])
    expect(body.flows).toEqual([{ id: flowId, name: flowName }])
    expect(body.agents).toEqual([{ id: agentId, name: agentName }])
    expect(body.teams).toEqual([{ id: inboxTeamId, name: teamName }])
    expect(sortBy(body.customFields, (field) => field.id)).toEqual(
      sortBy(
        fields
          .filter((field) => ["shortText", "longText", "number"].includes(field.type))
          .map((field) => ({ id: field.id, name: field.name, type: field.type })),
        (field) => field.id,
      ),
    )
    expect(sortBy(body.users, (user) => user.id)).toEqual(
      sortBy(
        [
          { id: tenant.userId, name: "Parity p107-dp-dest" },
          { id: agentUserId, name: "Parity Agent Member" },
        ],
        (user) => user.id,
      ),
    )
  })
})
