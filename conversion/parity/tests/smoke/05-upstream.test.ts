import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { createTenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueHost, uniqueId } from "../../src/unique.ts"

// A valid System One Compatible answer to the conformance test of
// decisionConnections.test (three questions: choice, score, noul).
const conformanceAnswer = {
  model: "parity-model",
  answers: {
    choice_test: { type: "choice", choice: "one", confidence: 0.9 },
    score_test: { type: "score", score: 0, confidence: 0.8 },
    noul_test: { type: "noul", probability: 0.25, confidence: 0.7 },
  },
}

describe("smoke: fake upstream round trip (DoH + TLS + pinned dial)", () => {
  it("tests an AI Router connection against a scripted provider", async () => {
    const normalizer = new Normalizer()
    const tenant = await createTenant({ label: "upstream" })
    const api = apiClient(tenant.token)
    const host = uniqueHost("router")
    const credential = `sk-parity-${uniqueId()}`

    await fake.script({
      match: { host, method: "POST", path: "/v1" },
      respond: { status: 200, json: conformanceAnswer },
    })

    const created = await api.post<{ id: string }>("/v1/decision-connections", {
      name: "Parity router",
      providerKind: "systemOneCompatible",
      endpoint: `https://${host}/v1`,
      modelCatalog: ["parity-model"],
      defaultModel: "parity-model",
      credential,
    })
    expect(created.status).toBe(201)
    expect(
      normalizer.normalize(created.json, { replace: { [host]: "<host>", [`https://${host}/v1`]: "https://<host>/v1" } }),
    ).toMatchSnapshot("created connection")

    const tested = await api.post(`/v1/decision-connections/${created.json.id}/test`, {})
    expect(tested.status).toBe(200)
    expect(tested.json).toMatchObject({ code: "ok", model: "parity-model", status: "passed" })
    expect(typeof (tested.json as { latencyMs: unknown }).latencyMs).toBe("number")

    // The SSRF guard resolved the host through DoH on 1.1.1.1 (the original
    // also makes a plain DNS lookup when it dials; that is not asserted) ...
    const doh = (await fake.requests({ kind: "dns", host })).filter(
      (record) => record.dnsTransport === "doh",
    )
    expect(doh.length).toBeGreaterThanOrEqual(1)
    expect(doh.every((record) => record.dnsType === "A")).toBe(true)
    const dohHttp = (await fake.requests({ kind: "http", host: "1.1.1.1", pathPrefix: "/dns-query" })).filter(
      (record) => record.query.name === host,
    )
    expect(dohHttp.length).toBeGreaterThanOrEqual(1)
    expect(dohHttp[0]).toMatchObject({ scheme: "https", method: "GET", query: { name: host, type: "A" } })

    // ... and the provider call reached the fake over TLS with the test CA.
    const calls = await fake.requests({ kind: "http", host })
    expect(calls).toHaveLength(1)
    const [call] = calls
    expect(call).toMatchObject({ scheme: "https", method: "POST", path: "/v1", unmatched: false })
    expect(call?.headers.authorization).toBe(`Bearer ${credential}`)
    expect(call?.headers["content-type"]).toMatch(/^application\/json/)
    const sent = JSON.parse(call?.bodyText ?? "{}") as Record<string, unknown>
    expect(sent.model).toBe("parity-model")
    expect(Object.keys(sent.questions as object).sort()).toEqual(["choice_test", "noul_test", "score_test"])
    expect(sent.state).toEqual({ currentMessage: "Decision connection conformance test" })
  })

  it("reports a failed test when the provider answers 500", async () => {
    const tenant = await createTenant({ label: "upstream-fail" })
    const api = apiClient(tenant.token)
    const host = uniqueHost("router-fail")
    await fake.script({
      match: { host, method: "POST", path: "/v1" },
      respond: { status: 500, json: { error: "boom" } },
    })
    const created = await api.post<{ id: string }>("/v1/decision-connections", {
      name: "Parity failing router",
      providerKind: "systemOneCompatible",
      endpoint: `https://${host}/v1`,
      modelCatalog: ["parity-model"],
      credential: `sk-parity-${uniqueId()}`,
    })
    expect(created.status).toBe(201)
    const tested = await api.post(`/v1/decision-connections/${created.json.id}/test`, {})
    expect(tested.status).toBe(200)
    expect(tested.json).toMatchObject({ code: "failed", model: "parity-model", status: "failed" })
    expect(await fake.requests({ kind: "http", host })).toHaveLength(1)
  })

  it("rejects an endpoint whose DNS answer is a private address (SSRF guard)", async () => {
    const tenant = await createTenant({ label: "upstream-ssrf" })
    const host = uniqueHost("router-private")
    await fake.dns({ name: host, a: ["10.0.0.7"] })
    const created = await apiClient(tenant.token).post("/v1/decision-connections", {
      name: "Parity private router",
      providerKind: "systemOneCompatible",
      endpoint: `https://${host}/v1`,
      modelCatalog: ["parity-model"],
      credential: `sk-parity-${uniqueId()}`,
    })
    expect(created.status).toBe(400)
    expect((created.json as { code?: string }).code).toBe("unsafeDecisionEndpoint")
    expect(await fake.requests({ kind: "http", host })).toHaveLength(0)
  })
})
