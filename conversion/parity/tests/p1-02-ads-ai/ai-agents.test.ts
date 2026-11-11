// P1.2 — aiAgents.* (scope `automation`). Spec: conversion/specs/P1.2.md.

import { describe, expect, it } from "vitest"
import { apiClient } from "../../src/http.ts"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  AUTOMATION_SCOPE_MESSAGE,
  aiTenant,
  expectError,
  MISSING_ID,
  READ_ONLY_MESSAGE,
  restrictedClients,
} from "./ai-helpers.ts"

type Agent = { id: string; name: string; isDefault: boolean; createdAt: string; updatedAt: string; [key: string]: unknown }

const agentInput = (overrides: Record<string, unknown> = {}) => ({
  name: `Parity agent ${uniqueId()}`,
  prompt: "You answer questions about parity.",
  messages: [{ role: "user", content: "Hello" }],
  models: [{ provider: "openai", model: "gpt-4o-mini" }],
  temperature: 0.4,
  maxOutputTokens: 500,
  tools: [],
  webSearchAuthorizedDomains: [{ value: " Docs.Example.com " }],
  isDefault: false,
  ...overrides,
})

const agentShape = (input: ReturnType<typeof agentInput>, workspace: string) => ({
  id: "<id:1>",
  workspaceId: workspace,
  createdAt: "<ts>",
  updatedAt: "<ts>",
  name: input.name,
  prompt: input.prompt,
  messages: input.messages,
  models: input.models,
  temperature: input.temperature,
  maxOutputTokens: input.maxOutputTokens,
  tools: input.tools,
  webSearchAuthorizedDomains: ["docs.example.com"],
  isDefault: input.isDefault,
  isRichResponse: false,
})

describe("P1.2 aiAgents", () => {
  it("create returns 201 with the public agent shape; get and list return the same agent", async () => {
    const normalizer = new Normalizer()
    const { api, workspaceId } = await aiTenant("agent-create")
    const input = agentInput()
    const created = await api.post<Agent>("/v1/ai-agents", input)
    expect(created.status, created.text).toBe(201)
    const expected = agentShape(input, "<ws>")
    const replace = { replace: { [workspaceId]: "<ws>" } }
    expect(normalizer.normalize(created.json, replace)).toEqual(expected)
    // The private AI action fields never reach the public API.
    expect(created.json).not.toHaveProperty("actionPrompt")
    expect(created.json).not.toHaveProperty("actionRules")

    const got = await api.get<Agent>(`/v1/ai-agents/${created.json.id}`)
    expect(got.status).toBe(200)
    expect(normalizer.normalize(got.json, replace)).toEqual(expected)

    const listed = await api.get<{ data: Agent[]; pageCount: number }>("/v1/ai-agents")
    expect(listed.status).toBe(200)
    expect(normalizer.normalize(listed.json, replace)).toEqual({ data: [expected], pageCount: 1 })
  })

  it("list is empty for a new workspace and pages newest first", async () => {
    const { api } = await aiTenant("agent-list")
    const empty = await api.get("/v1/ai-agents")
    expect(empty.status).toBe(200)
    expect(empty.json).toEqual({ data: [], pageCount: 0 })

    const names: string[] = []
    for (let index = 0; index < 3; index += 1) {
      const input = agentInput({ name: `Agent ${index} ${uniqueId()}` })
      names.push(input.name)
      expect((await api.post("/v1/ai-agents", input)).status).toBe(201)
    }
    const page1 = await api.get<{ data: Agent[]; pageCount: number }>("/v1/ai-agents", {
      query: { page: 1, perPage: 2 },
    })
    expect(page1.status).toBe(200)
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data.map((agent) => agent.name)).toEqual([names[2], names[1]])
    const page2 = await api.get<{ data: Agent[]; pageCount: number }>("/v1/ai-agents", {
      query: { page: 2, perPage: 2 },
    })
    expect(page2.json.data.map((agent) => agent.name)).toEqual([names[0]])
    expectError(await api.get("/v1/ai-agents", { query: { perPage: 51 } }), 422, "invalidRequestData")
    expectError(await api.get("/v1/ai-agents", { query: { page: 0 } }), 422, "invalidRequestData")
  })

  it("update changes only the given fields", async () => {
    const normalizer = new Normalizer()
    const { api, workspaceId } = await aiTenant("agent-update")
    const input = agentInput()
    const created = await api.post<Agent>("/v1/ai-agents", input)
    expect(created.status).toBe(201)
    const name = `Renamed ${uniqueId()}`
    const updated = await api.patch<Agent>(`/v1/ai-agents/${created.json.id}`, {
      name,
      isRichResponse: true,
      webSearchAuthorizedDomains: [{ value: "Shop.Example.org" }, { value: "shop.example.org" }],
    })
    expect(updated.status, updated.text).toBe(200)
    expect(normalizer.normalize(updated.json, { replace: { [workspaceId]: "<ws>" } })).toEqual({
      ...agentShape(input, "<ws>"),
      name,
      isRichResponse: true,
      webSearchAuthorizedDomains: ["shop.example.org"],
    })
    const got = await api.get<Agent>(`/v1/ai-agents/${created.json.id}`)
    expect(got.json).toEqual(updated.json)

    // An empty body changes nothing and returns the stored agent.
    const unchanged = await api.patch<Agent>(`/v1/ai-agents/${created.json.id}`, {})
    expect(unchanged.status).toBe(200)
    expect(unchanged.json).toEqual(updated.json)
  })

  it("a new default agent clears the previous default", async () => {
    const { api } = await aiTenant("agent-default")
    const first = await api.post<Agent>("/v1/ai-agents", agentInput({ isDefault: true }))
    const second = await api.post<Agent>("/v1/ai-agents", agentInput({ isDefault: true }))
    expect(first.status).toBe(201)
    expect(second.status).toBe(201)
    expect((await api.get<Agent>(`/v1/ai-agents/${first.json.id}`)).json.isDefault).toBe(false)
    expect((await api.get<Agent>(`/v1/ai-agents/${second.json.id}`)).json.isDefault).toBe(true)

    // The same rule applies on update.
    const updated = await api.patch<Agent>(`/v1/ai-agents/${first.json.id}`, { isDefault: true })
    expect(updated.status).toBe(200)
    expect(updated.json.isDefault).toBe(true)
    expect((await api.get<Agent>(`/v1/ai-agents/${second.json.id}`)).json.isDefault).toBe(false)
  })

  it("delete returns 204 and the agent is gone; a missing id is also 204", async () => {
    const { api } = await aiTenant("agent-delete")
    const created = await api.post<Agent>("/v1/ai-agents", agentInput())
    const deleted = await api.delete(`/v1/ai-agents/${created.json.id}`)
    expect(deleted.status).toBe(204)
    expect(deleted.text).toBe("")
    expectError(await api.get(`/v1/ai-agents/${created.json.id}`), 404, "notFound")
    expect((await api.get("/v1/ai-agents")).json).toEqual({ data: [], pageCount: 0 })
    expect((await api.delete(`/v1/ai-agents/${MISSING_ID}`)).status).toBe(204)
  })

  it("agents of another workspace are not visible", async () => {
    const owner = await aiTenant("agent-owner")
    const other = await aiTenant("agent-other")
    const created = await owner.api.post<Agent>("/v1/ai-agents", agentInput())
    expectError(await other.api.get(`/v1/ai-agents/${created.json.id}`), 404, "notFound")
    expectError(await other.api.patch(`/v1/ai-agents/${created.json.id}`, { name: "x" }), 404, "notFound")
    expect((await other.api.delete(`/v1/ai-agents/${created.json.id}`)).status).toBe(204)
    expect((await owner.api.get(`/v1/ai-agents/${created.json.id}`)).status).toBe(200)
  })

  it("rejects invalid input with 422 invalidRequestData and unknown ids with 404", async () => {
    const { api } = await aiTenant("agent-invalid")
    const { prompt: _prompt, ...withoutPrompt } = agentInput()
    const missing = await api.post<{ data: { issues: Array<{ path?: unknown[] }> } }>("/v1/ai-agents", withoutPrompt)
    expectError(missing, 422, "invalidRequestData")
    expect(missing.json.data.issues.some((issue) => issue.path?.[0] === "prompt")).toBe(true)
    // The private AI action fields are not accepted (strict schema).
    expectError(await api.post("/v1/ai-agents", agentInput({ actionPrompt: "secret" })), 422, "invalidRequestData")
    expectError(await api.post("/v1/ai-agents", agentInput({ temperature: 2.5 })), 422, "invalidRequestData")
    expectError(await api.post("/v1/ai-agents", agentInput({ maxOutputTokens: 32_769 })), 422, "invalidRequestData")
    expectError(await api.post("/v1/ai-agents", agentInput({ name: "  " })), 422, "invalidRequestData")
    expectError(
      await api.post("/v1/ai-agents", agentInput({ models: [{ provider: "openai", model: "no-such-model" }] })),
      422,
      "invalidRequestData",
    )
    expectError(
      await api.post("/v1/ai-agents", agentInput({ messages: [{ role: "robot", content: "x" }] })),
      422,
      "invalidRequestData",
    )
    expectError(
      await api.post("/v1/ai-agents", agentInput({ webSearchAuthorizedDomains: [{ value: "https://example.com/a" }] })),
      422,
      "invalidRequestData",
    )
    expectError(await api.get("/v1/ai-agents/not-a-number"), 422, "invalidRequestData")
    expectError(await api.get(`/v1/ai-agents/${MISSING_ID}`), 404, "notFound")
    expectError(await api.patch(`/v1/ai-agents/${MISSING_ID}`, { name: "x" }), 404, "notFound")
    expectError(await api.patch(`/v1/ai-agents/${MISSING_ID}`, { temperature: -1 }), 422, "invalidRequestData")
  })

  it("update does not reject the private action keys (the strict schema is intersected with the id)", async () => {
    // Finding: create rejects `actionPrompt`/`actionRules` (strict), but on
    // PATCH the strict object is `.and()`-ed with `{id}`; the original lets
    // the keys through to the service. The response still hides them.
    const { api } = await aiTenant("agent-update-private")
    expectError(await api.patch(`/v1/ai-agents/${MISSING_ID}`, { actionRules: [] }), 404, "notFound")
    const created = await api.post<Agent>("/v1/ai-agents", agentInput())
    const updated = await api.patch<Agent>(`/v1/ai-agents/${created.json.id}`, { actionPrompt: "private" })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).not.toHaveProperty("actionPrompt")
    expect(updated.json).not.toHaveProperty("actionRules")
  })

  it("auth: 401 without a token, 403 without the automation scope, 403 for read-only writes", async () => {
    const owner = await aiTenant("agent-auth")
    const created = await owner.api.post<Agent>("/v1/ai-agents", agentInput())
    const id = created.json.id
    const none = apiClient("")
    const { wrongScope, readOnly } = await restrictedClients()
    const calls = [
      (api: typeof none) => api.get("/v1/ai-agents"),
      (api: typeof none) => api.get(`/v1/ai-agents/${id}`),
      (api: typeof none) => api.post("/v1/ai-agents", agentInput()),
      (api: typeof none) => api.patch(`/v1/ai-agents/${id}`, { name: "x" }),
      (api: typeof none) => api.delete(`/v1/ai-agents/${id}`),
    ]
    for (const call of calls) {
      expectError(await call(none), 401, "INVALID_CHATBOT_TOKEN")
      expectError(await call(wrongScope), 403, "FORBIDDEN", AUTOMATION_SCOPE_MESSAGE)
    }
    expect((await readOnly.get("/v1/ai-agents")).status).toBe(200)
    expectError(await readOnly.get(`/v1/ai-agents/${MISSING_ID}`), 404, "notFound")
    expectError(await readOnly.post("/v1/ai-agents", agentInput()), 403, "FORBIDDEN", READ_ONLY_MESSAGE)
    expectError(await readOnly.patch(`/v1/ai-agents/${MISSING_ID}`, { name: "x" }), 403, "FORBIDDEN", READ_ONLY_MESSAGE)
    expectError(await readOnly.delete(`/v1/ai-agents/${MISSING_ID}`), 403, "FORBIDDEN", READ_ONLY_MESSAGE)
    // Nothing changed for the owner.
    expect((await owner.api.get(`/v1/ai-agents/${id}`)).status).toBe(200)
  })
})
