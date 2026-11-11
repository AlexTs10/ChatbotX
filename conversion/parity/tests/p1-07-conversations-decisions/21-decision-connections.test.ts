// decisionConnections.create / list / update / setEnabled / test (scope
// `automation`). The endpoint goes through the SSRF guard (DoH on 1.1.1.1 ->
// fake upstream); `test` calls the provider through the fake upstream. The
// credential is write-only: it is read back only as the `Authorization`
// header of the provider call.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueHost, uniqueId } from "../../src/unique.ts"
import {
  type Connection,
  conformanceAnswer,
  createCompatibleConnection,
  expectInvalidAt,
  providerCalls,
  scriptProvider,
} from "./helpers-decisions.ts"
import { expectError, expectIsoNear, newTenant, UNKNOWN_ID } from "./helpers.ts"

type ConnectionList = { data: Connection[]; pageCount: number }
type TestResult = { code: string; latencyMs: number; model: string; status: "passed" | "failed" }

const PATH = "/v1/decision-connections"

const dohQueriesFor = async (host: string) =>
  (await fake.requests({ kind: "http", host: "1.1.1.1", pathPrefix: "/dns-query" })).filter(
    (record) => record.query.name === host,
  )

describe("decisionConnections.create", () => {
  it("creates a System One Compatible connection (201): credential-safe shape, endpoint normalized", async () => {
    const n = new Normalizer()
    const { api } = await newTenant("p107-dc-create")
    const host = uniqueHost("router")
    const created = await api.post<Connection>(PATH, {
      name: "  Parity router  ",
      providerKind: "systemOneCompatible",
      endpoint: `https://${host.toUpperCase()}/v1/decide///`,
      modelCatalog: [" model-a ", "model-b", "model-a"],
      defaultModel: "model-b",
      credential: `sk-parity-${uniqueId()}`,
    })
    expect(created.status, created.text).toBe(201)
    expect(n.normalize(created.json)).toEqual({
      id: "<id:1>",
      name: "Parity router",
      providerKind: "systemOneCompatible",
      endpoint: `https://${host}/v1/decide`,
      modelCatalog: ["model-a", "model-b"],
      defaultModel: "model-b",
      status: "enabled",
      credentialConfigured: true,
      lastTest: { status: null, testedAt: null },
    })
    expect(created.text).not.toContain("sk-parity-")
    // The guard resolved the host through DoH; no HTTP call reached the host.
    expect((await dohQueriesFor(host)).length).toBeGreaterThanOrEqual(1)
    expect(await providerCalls(host)).toHaveLength(0)
  })

  it("built-in providers store no endpoint; defaultModel is optional", async () => {
    const { api } = await newTenant("p107-dc-builtin")
    for (const providerKind of ["typesafe", "openrouterDecision"] as const) {
      const created = await api.post<Connection>(PATH, {
        name: `Parity ${providerKind} ${uniqueId()}`,
        providerKind,
        endpoint: "https://ignored.parity.test/v1",
        modelCatalog: ["m1"],
        credential: "sk-builtin",
      })
      expect(created.status, created.text).toBe(201)
      expect(created.json).toMatchObject({ providerKind, endpoint: null, defaultModel: null, modelCatalog: ["m1"] })
    }
  })

  it("rejects unsafe or non-canonical endpoints", async () => {
    const { api } = await newTenant("p107-dc-endpoint")
    const base = { name: `Bad ${uniqueId()}`, providerKind: "systemOneCompatible", modelCatalog: ["m"], credential: "k" }
    const privateHost = uniqueHost("router-private")
    await fake.dns({ name: privateHost, a: ["10.0.0.7"] })
    const unsafe = [`https://${privateHost}/v1`, "https://127.0.0.1/v1", "https://10.1.2.3/v1", "https://localhost/v1"]
    for (const endpoint of unsafe) {
      const response = await api.post(PATH, { ...base, endpoint })
      expectError(response, 400, "unsafeDecisionEndpoint", {
        message: "Decision provider endpoint is not publicly reachable",
      })
    }
    expect(await providerCalls(privateHost)).toHaveLength(0)
    const host = uniqueHost("router-canon")
    for (const endpoint of [
      `http://${host}/v1`,
      `https://${host}/v1?x=1`,
      `https://${host}/v1#frag`,
      `https://user:pw@${host}/v1`,
    ]) {
      const response = await api.post(PATH, { ...base, endpoint })
      expectError(response, 400, "invalidDecisionEndpoint", {
        defined: true,
        message: "Decision provider endpoint must be a canonical HTTPS URL",
      })
    }
    expectError(await api.post(PATH, { ...base, endpoint: "not a url" }), 400, "invalidDecisionEndpoint", {
      message: "Decision provider endpoint must be a valid HTTPS URL",
    })
    for (const endpoint of [null, ""]) {
      expectError(await api.post(PATH, { ...base, endpoint }), 400, "invalidDecisionEndpoint", {
        message: "System One Compatible requires an endpoint",
      })
    }
    expectError(await api.post(PATH, base), 400, "invalidDecisionEndpoint")
    expect((await api.get<ConnectionList>(PATH)).json.data).toEqual([])
  })

  it("rejects a default model outside the catalog (400 invalidDecisionModel)", async () => {
    const { api } = await newTenant("p107-dc-model")
    const response = await api.post(PATH, {
      name: `Model ${uniqueId()}`,
      providerKind: "typesafe",
      modelCatalog: ["a"],
      defaultModel: "b",
      credential: "k",
    })
    expectError(response, 400, "invalidDecisionModel", {
      defined: true,
      message: "Default decision model must be in the connection catalog",
    })
  })

  it("validates the input (422 invalidRequestData)", async () => {
    const { api } = await newTenant("p107-dc-invalid")
    const ok = { name: `V ${uniqueId()}`, providerKind: "typesafe", modelCatalog: ["m"], credential: "k" }
    expectInvalidAt(await api.post(PATH, { ...ok, credential: undefined }), "credential")
    expectInvalidAt(await api.post(PATH, { ...ok, credential: "   " }), "credential")
    expectInvalidAt(await api.post(PATH, { ...ok, modelCatalog: [] }), "modelCatalog")
    expectInvalidAt(await api.post(PATH, { ...ok, modelCatalog: [" "] }), "modelCatalog.0")
    expectInvalidAt(await api.post(PATH, { ...ok, name: " " }), "name")
    expectInvalidAt(await api.post(PATH, { ...ok, name: "x".repeat(161) }), "name")
    expectInvalidAt(await api.post(PATH, { ...ok, providerKind: "openai" }), "providerKind")
    expectError(await api.post(PATH, { ...ok, status: "enabled" }), 422, "invalidRequestData")
  })
})

describe("decisionConnections.list", () => {
  it("lists newest first, paged in memory (pageCount >= 1), never with a credential", async () => {
    const { api } = await newTenant("p107-dc-list")
    expect((await api.get<ConnectionList>(PATH)).json).toEqual({ data: [], pageCount: 1 })
    const ids: string[] = []
    for (let index = 0; index < 3; index += 1) {
      ids.push((await createCompatibleConnection(api)).connection.id)
    }
    const all = await api.get<ConnectionList>(PATH)
    expect(all.status, all.text).toBe(200)
    expect(all.json.pageCount).toBe(1)
    expect(all.json.data.map((connection) => connection.id)).toEqual([...ids].reverse())
    expect(all.text).not.toContain("sk-parity-")
    expect(all.json.data.every((connection) => connection.credentialConfigured === true)).toBe(true)

    const page1 = await api.get<ConnectionList>(PATH, { query: { page: 1, perPage: 2 } })
    expect(page1.json).toMatchObject({ pageCount: 2 })
    expect(page1.json.data.map((connection) => connection.id)).toEqual([ids[2], ids[1]])
    const page2 = await api.get<ConnectionList>(PATH, { query: { page: 2, perPage: 2 } })
    expect(page2.json.data.map((connection) => connection.id)).toEqual([ids[0]])
    const page9 = await api.get<ConnectionList>(PATH, { query: { page: 9, perPage: 2 } })
    expect(page9.json).toEqual({ data: [], pageCount: 2 })

    const other = await newTenant("p107-dc-list-other")
    expect((await other.api.get<ConnectionList>(PATH)).json.data).toEqual([])
    expectInvalidAt(await api.get(PATH, { query: { perPage: 51 } }), "perPage")
    expectInvalidAt(await api.get(PATH, { query: { page: 0 } }), "page")
  })
})

describe("decisionConnections.update", () => {
  it("replaces the settings; without a credential the old secret is kept", async () => {
    const { api } = await newTenant("p107-dc-update")
    const created = await createCompatibleConnection(api)
    const newHost = uniqueHost("router-moved")
    const newModel = `parity-new-${uniqueId()}`
    const updated = await api.put<Connection>(`${PATH}/${created.connection.id}`, {
      name: `Renamed ${uniqueId()}`,
      providerKind: "systemOneCompatible",
      endpoint: `https://${newHost}/decide/`,
      modelCatalog: [newModel, created.model],
      defaultModel: newModel,
    })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).toEqual({
      ...created.connection,
      name: updated.json.name,
      endpoint: `https://${newHost}/decide`,
      modelCatalog: [newModel, created.model],
      defaultModel: newModel,
    })
    const listed = await api.get<ConnectionList>(PATH)
    expect(listed.json.data).toEqual([updated.json])

    await scriptProvider(newHost, "/decide", { status: 200, json: conformanceAnswer(newModel) })
    const tested = await api.post<TestResult>(`${PATH}/${created.connection.id}/test`, {})
    expect(tested.status, tested.text).toBe(200)
    expect(tested.json).toMatchObject({ code: "ok", status: "passed", model: newModel })
    const calls = await providerCalls(newHost)
    expect(calls).toHaveLength(1)
    expect(calls[0]?.headers.authorization).toBe(`Bearer ${created.credential}`)
    expect(await providerCalls(created.host)).toHaveLength(0)
  })

  it("a new credential replaces the secret; an empty one keeps it", async () => {
    const { api } = await newTenant("p107-dc-rotate")
    const created = await createCompatibleConnection(api)
    const body = {
      name: created.connection.name,
      providerKind: "systemOneCompatible",
      endpoint: created.endpoint,
      modelCatalog: [created.model],
      defaultModel: created.model,
    }
    const rotated = `sk-rotated-${uniqueId()}`
    expect((await api.put(`${PATH}/${created.connection.id}`, { ...body, credential: rotated })).status).toBe(200)
    expect((await api.put(`${PATH}/${created.connection.id}`, { ...body, credential: "" })).status).toBe(200)
    await scriptProvider(created.host, "/v1/decide", { status: 200, json: conformanceAnswer(created.model) })
    const tested = await api.post<TestResult>(`${PATH}/${created.connection.id}/test`, {})
    expect(tested.json.status).toBe("passed")
    const calls = await providerCalls(created.host)
    expect(calls).toHaveLength(1)
    expect(calls[0]?.headers.authorization).toBe(`Bearer ${rotated}`)
  })

  it("switching to a built-in provider drops the endpoint; a missing defaultModel becomes null", async () => {
    const { api } = await newTenant("p107-dc-kind")
    const created = await createCompatibleConnection(api)
    const updated = await api.put<Connection>(`${PATH}/${created.connection.id}`, {
      name: created.connection.name,
      providerKind: "openrouterDecision",
      endpoint: created.endpoint,
      modelCatalog: ["router-model"],
    })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).toMatchObject({ providerKind: "openrouterDecision", endpoint: null, defaultModel: null })
  })

  it("is 404 for an unknown or foreign id (before endpoint checks) and checks model / endpoint", async () => {
    const { api } = await newTenant("p107-dc-update-bad")
    const other = await newTenant("p107-dc-update-other")
    const created = await createCompatibleConnection(api)
    const body = {
      name: "x",
      providerKind: "systemOneCompatible",
      endpoint: "http://not-https.parity.test/v1",
      modelCatalog: ["m"],
    }
    expectError(await api.put(`${PATH}/${UNKNOWN_ID}`, body), 404, "notFound", {
      defined: true,
      message: "Decision connection not found",
    })
    expectError(await other.api.put(`${PATH}/${created.connection.id}`, body), 404, "notFound")
    expectError(await api.put(`${PATH}/${created.connection.id}`, body), 400, "invalidDecisionEndpoint")
    expectError(
      await api.put(`${PATH}/${created.connection.id}`, { ...body, endpoint: created.endpoint, defaultModel: "z" }),
      400,
      "invalidDecisionModel",
    )
    expectInvalidAt(await api.put(`${PATH}/abc`, body), "id")
    expectInvalidAt(await api.put(`${PATH}/${created.connection.id}`, { ...body, modelCatalog: [] }), "modelCatalog")
    expect((await api.get<ConnectionList>(PATH)).json.data).toEqual([created.connection])
  })
})

describe("decisionConnections.setEnabled", () => {
  it("disables and re-enables (200, connection shape)", async () => {
    const { api } = await newTenant("p107-dc-status")
    const created = await createCompatibleConnection(api)
    const disabled = await api.patch<Connection>(`${PATH}/${created.connection.id}/status`, { enabled: false })
    expect(disabled.status, disabled.text).toBe(200)
    expect(disabled.json).toEqual({ ...created.connection, status: "disabled" })
    expect((await api.get<ConnectionList>(PATH)).json.data[0]?.status).toBe("disabled")
    const enabled = await api.patch<Connection>(`${PATH}/${created.connection.id}/status`, { enabled: true })
    expect(enabled.json).toEqual(created.connection)
  })

  it("is 404 for an unknown or foreign id; strict body", async () => {
    const { api } = await newTenant("p107-dc-status-bad")
    const other = await newTenant("p107-dc-status-other")
    const created = await createCompatibleConnection(api)
    expectError(await api.patch(`${PATH}/${UNKNOWN_ID}/status`, { enabled: false }), 404, "notFound", {
      defined: true,
      message: "Decision connection not found",
    })
    expectError(await other.api.patch(`${PATH}/${created.connection.id}/status`, { enabled: false }), 404, "notFound")
    expectInvalidAt(await api.patch(`${PATH}/${created.connection.id}/status`, {}), "enabled")
    expectInvalidAt(await api.patch(`${PATH}/${created.connection.id}/status`, { enabled: "no" }), "enabled")
    expectError(
      await api.patch(`${PATH}/${created.connection.id}/status`, { enabled: false, name: "x" }),
      422,
      "invalidRequestData",
    )
    expect((await api.get<ConnectionList>(PATH)).json.data[0]?.status).toBe("enabled")
  })
})

describe("decisionConnections.test", () => {
  it("passes against a conformant provider; records lastTest; the request carries the contract", async () => {
    const { api } = await newTenant("p107-dc-test")
    const testStart = Date.now()
    const created = await createCompatibleConnection(api, { modelCatalog: ["first-model", "second-model"], defaultModel: null })
    await scriptProvider(created.host, "/v1/decide", { status: 200, json: conformanceAnswer("first-model") })
    const tested = await api.post<TestResult>(`${PATH}/${created.connection.id}/test`, {})
    expect(tested.status, tested.text).toBe(200)
    expect(Object.keys(tested.json).sort()).toEqual(["code", "latencyMs", "model", "status"])
    expect(tested.json).toMatchObject({ code: "ok", status: "passed", model: "first-model" })
    expect(Number.isInteger(tested.json.latencyMs)).toBe(true)

    const [call, ...more] = await providerCalls(created.host)
    expect(more).toHaveLength(0)
    expect(call).toMatchObject({ scheme: "https", method: "POST", path: "/v1/decide" })
    expect(call?.headers.authorization).toBe(`Bearer ${created.credential}`)
    expect(call?.headers["content-type"]).toMatch(/^application\/json/)
    expect(JSON.parse(call?.bodyText ?? "{}")).toEqual({
      model: "first-model",
      questions: {
        choice_test: {
          instructions: "Classify the test input.",
          key: "choice_test",
          label: "Choice",
          options: [
            { description: "First test choice.", label: "One", value: "one" },
            { description: "Second test choice.", label: "Two", value: "two" },
          ],
          type: "choice",
        },
        score_test: {
          instructions: "Score the test input.",
          key: "score_test",
          label: "Score",
          levels: [
            { description: "Low test score.", label: "1", value: 1 },
            { description: "High test score.", label: "2", value: 2 },
          ],
          type: "score",
        },
        noul_test: { instructions: "Evaluate the test input.", key: "noul_test", label: "Noul", type: "noul" },
      },
      state: { currentMessage: "Decision connection conformance test" },
    })

    const listed = await api.get<ConnectionList>(PATH)
    const [row] = listed.json.data
    expect(row?.lastTest.status).toBe("passed")
    expectIsoNear(row?.lastTest.testedAt, testStart)
  })

  it("a second test within 30 s is throttled (429) and makes no provider call", async () => {
    const { api } = await newTenant("p107-dc-throttle")
    const created = await createCompatibleConnection(api)
    await scriptProvider(created.host, "/v1/decide", { status: 200, json: conformanceAnswer(created.model) })
    expect((await api.post<TestResult>(`${PATH}/${created.connection.id}/test`, {})).json.status).toBe("passed")
    const again = await api.post(`${PATH}/${created.connection.id}/test`, {})
    expectError(again, 429, "decisionConnectionTestThrottled", {
      defined: true,
      message: "Decision connection test is already running or was run recently",
    })
    expect(await providerCalls(created.host)).toHaveLength(1)
    // The throttle is per connection: another connection still tests.
    const second = await createCompatibleConnection(api)
    await scriptProvider(second.host, "/v1/decide", { status: 200, json: conformanceAnswer(second.model) })
    expect((await api.post<TestResult>(`${PATH}/${second.connection.id}/test`, {})).status).toBe(200)
  })

  it("reports failed (200) for a non-conformant answer, an error status and a bad body", async () => {
    const { api } = await newTenant("p107-dc-test-fail")
    const answers = [
      { status: 200, json: { model: "m", answers: { choice_test: { type: "choice", choice: "one" } } } },
      { status: 503, json: { error: "down" } },
      { status: 200, text: "not json" },
    ]
    for (const respond of answers) {
      const created = await createCompatibleConnection(api)
      await scriptProvider(created.host, "/v1/decide", respond)
      const tested = await api.post<TestResult>(`${PATH}/${created.connection.id}/test`, {})
      expect(tested.status, tested.text).toBe(200)
      expect(tested.json).toMatchObject({ code: "failed", status: "failed", model: created.model })
      expect(await providerCalls(created.host)).toHaveLength(1)
      const listed = await api.get<ConnectionList>(PATH)
      expect(listed.json.data.find((row) => row.id === created.connection.id)?.lastTest.status).toBe("failed")
    }
  })

  it("a redirect answer is followed with a GET (same pinned address) and the test fails", async () => {
    const { api } = await newTenant("p107-dc-test-redirect")
    const created = await createCompatibleConnection(api)
    const target = uniqueHost("redirect-target")
    await scriptProvider(created.host, "/v1/decide", {
      status: 302,
      headers: { location: `https://${target}/next` },
      text: "",
    })
    await fake.script({ match: { host: target, method: "GET", path: "/next" }, respond: { status: 404, text: "no" } })
    const tested = await api.post<TestResult>(`${PATH}/${created.connection.id}/test`, {})
    expect(tested.status, tested.text).toBe(200)
    expect(tested.json).toMatchObject({ code: "failed", status: "failed" })
    expect(await providerCalls(created.host)).toHaveLength(1)
    const followed = await fake.requests({ kind: "http", host: target })
    expect(followed.map((record) => ({ method: record.method, path: record.path }))).toEqual([
      { method: "GET", path: "/next" },
    ])
  })

  it("re-checks the address at test time: a host that now resolves privately fails without a call", async () => {
    const { api } = await newTenant("p107-dc-rebind")
    const created = await createCompatibleConnection(api)
    await fake.dns({ name: created.host, a: ["192.168.10.20"] })
    const tested = await api.post<TestResult>(`${PATH}/${created.connection.id}/test`, {})
    expect(tested.status, tested.text).toBe(200)
    expect(tested.json).toMatchObject({ code: "failed", status: "failed" })
    expect(await providerCalls(created.host)).toHaveLength(0)
  })

  it("tests a disabled connection too", async () => {
    const { api } = await newTenant("p107-dc-test-disabled")
    const created = await createCompatibleConnection(api)
    expect((await api.patch(`${PATH}/${created.connection.id}/status`, { enabled: false })).status).toBe(200)
    await scriptProvider(created.host, "/v1/decide", { status: 200, json: conformanceAnswer(created.model) })
    const tested = await api.post<TestResult>(`${PATH}/${created.connection.id}/test`, {})
    expect(tested.json).toMatchObject({ status: "passed" })
    expect(await providerCalls(created.host)).toHaveLength(1)
  })

  it("built-in providers call their fixed endpoint with the criteria request format", async () => {
    const { api } = await newTenant("p107-dc-test-builtin")
    const cases = [
      { providerKind: "typesafe", host: "api.typesafe.ai", path: "/v1/systemone" },
      { providerKind: "openrouterDecision", host: "openrouter.ai", path: "/api/alpha/decisions" },
    ] as const
    for (const { providerKind, host, path } of cases) {
      const model = `parity-builtin-${uniqueId()}`
      const credential = `sk-builtin-${uniqueId()}`
      const created = await api.post<Connection>(PATH, {
        name: `Builtin ${uniqueId()}`,
        providerKind,
        modelCatalog: [model],
        credential,
      })
      expect(created.status, created.text).toBe(201)
      await scriptProvider(host, path, { status: 200, json: conformanceAnswer(model) }, { bodyIncludes: model })
      const tested = await api.post<TestResult>(`${PATH}/${created.json.id}/test`, {})
      expect(tested.json, tested.text).toMatchObject({ status: "passed", code: "ok", model })
      const calls = await providerCalls(host, model)
      expect(calls).toHaveLength(1)
      expect(calls[0]).toMatchObject({ scheme: "https", method: "POST", path })
      expect(calls[0]?.headers.authorization).toBe(`Bearer ${credential}`)
      const sent = JSON.parse(calls[0]?.bodyText ?? "{}") as {
        questions: Record<string, Record<string, unknown>>
      }
      expect(sent.questions.choice_test).toEqual({
        instructions: "Classify the test input.",
        key: "choice_test",
        label: "Choice",
        type: "choice",
        criteria: { one: "First test choice.", two: "Second test choice." },
      })
      expect(sent.questions.score_test).toEqual({
        instructions: "Score the test input.",
        key: "score_test",
        label: "Score",
        type: "score",
        criteria: ["Low test score.", "High test score."],
      })
      expect(sent.questions.noul_test).toEqual({
        instructions: "Evaluate the test input.",
        key: "noul_test",
        label: "Noul",
        type: "noul",
      })
    }
  })

  it("is 404 for an unknown or foreign id (no provider call)", async () => {
    const { api } = await newTenant("p107-dc-test-404")
    const other = await newTenant("p107-dc-test-404-other")
    const created = await createCompatibleConnection(api)
    expectError(await api.post(`${PATH}/${UNKNOWN_ID}/test`, {}), 404, "notFound", {
      defined: true,
      message: "Decision connection not found",
    })
    expectError(await other.api.post(`${PATH}/${created.connection.id}/test`, {}), 404, "notFound")
    expect(await providerCalls(created.host)).toHaveLength(0)
    expectInvalidAt(await api.post(`${PATH}/abc/test`, {}), "id")
    // The foreign attempt did not take the throttle slot.
    await scriptProvider(created.host, "/v1/decide", { status: 200, json: conformanceAnswer(created.model) })
    expect((await api.post<TestResult>(`${PATH}/${created.connection.id}/test`, {})).json.status).toBe("passed")
  })
})
