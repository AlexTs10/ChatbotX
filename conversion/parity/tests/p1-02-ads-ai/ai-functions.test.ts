// P1.2 — aiFunctions.* (scope `automation`). Spec: conversion/specs/P1.2.md.

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

type AiFunction = { id: string; name: string; [key: string]: unknown }

const functionInput = (overrides: Record<string, unknown> = {}) => ({
  name: `collect_email_${uniqueId()}`,
  purpose: "Call when the contact gives an email address.",
  dataCollect: [{ from: "email", to: "123456789012345678" }],
  outputMessage: "Thanks, we saved it.",
  ...overrides,
})

describe("P1.2 aiFunctions", () => {
  it("create returns 201 with the stored function; get and list return it", async () => {
    const normalizer = new Normalizer()
    const { api, workspaceId } = await aiTenant("fn-create")
    const input = functionInput()
    const created = await api.post<AiFunction>("/v1/ai-functions", input)
    expect(created.status, created.text).toBe(201)
    const replace = { replace: { [workspaceId]: "<ws>" }, keep: ["to"] }
    const expected = {
      id: "<id:1>",
      workspaceId: "<ws>",
      createdAt: "<ts>",
      updatedAt: "<ts>",
      name: input.name,
      purpose: input.purpose,
      dataCollect: input.dataCollect,
      outputMessage: input.outputMessage,
      triggerFlowId: null,
    }
    expect(normalizer.normalize(created.json, replace)).toEqual(expected)
    const got = await api.get(`/v1/ai-functions/${created.json.id}`)
    expect(got.status).toBe(200)
    expect(normalizer.normalize(got.json, replace)).toEqual(expected)
    const listed = await api.get("/v1/ai-functions")
    expect(listed.status).toBe(200)
    expect(normalizer.normalize(listed.json, replace)).toEqual({ data: [expected], pageCount: 1 })
  })

  it("create keeps omitted optional fields as null and trims text", async () => {
    const { api } = await aiTenant("fn-minimal")
    const name = `minimal_${uniqueId()}`
    const created = await api.post<AiFunction>("/v1/ai-functions", {
      name: `  ${name}  `,
      dataCollect: [],
    })
    expect(created.status, created.text).toBe(201)
    expect(created.json).toMatchObject({
      name,
      purpose: null,
      dataCollect: [],
      outputMessage: null,
      triggerFlowId: null,
    })
  })

  it("list is empty for a new workspace and pages newest first", async () => {
    const { api } = await aiTenant("fn-list")
    expect((await api.get("/v1/ai-functions")).json).toEqual({ data: [], pageCount: 0 })
    const names: string[] = []
    for (let index = 0; index < 3; index += 1) {
      const input = functionInput()
      names.push(input.name)
      expect((await api.post("/v1/ai-functions", input)).status).toBe(201)
    }
    const page1 = await api.get<{ data: AiFunction[]; pageCount: number }>("/v1/ai-functions", {
      query: { perPage: 2 },
    })
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data.map((fn) => fn.name)).toEqual([names[2], names[1]])
    const page2 = await api.get<{ data: AiFunction[]; pageCount: number }>("/v1/ai-functions", {
      query: { page: 2, perPage: 2 },
    })
    expect(page2.json.data.map((fn) => fn.name)).toEqual([names[0]])
    expectError(await api.get("/v1/ai-functions", { query: { perPage: 0 } }), 422, "invalidRequestData")
  })

  it("update (PUT) replaces the function", async () => {
    const normalizer = new Normalizer()
    const { api, workspaceId } = await aiTenant("fn-update")
    const created = await api.post<AiFunction>("/v1/ai-functions", functionInput())
    const next = functionInput({ purpose: null, outputMessage: "Done.", dataCollect: [{ from: "phone", to: "223456789012345678" }] })
    const updated = await api.put<AiFunction>(`/v1/ai-functions/${created.json.id}`, next)
    expect(updated.status, updated.text).toBe(200)
    expect(normalizer.normalize(updated.json, { replace: { [workspaceId]: "<ws>" }, keep: ["to"] })).toEqual({
      id: "<id:1>",
      workspaceId: "<ws>",
      createdAt: "<ts>",
      updatedAt: "<ts>",
      name: next.name,
      purpose: null,
      dataCollect: next.dataCollect,
      outputMessage: "Done.",
      triggerFlowId: null,
    })
    expect((await api.get(`/v1/ai-functions/${created.json.id}`)).json).toEqual(updated.json)
    // Keeping its own name is allowed.
    expect((await api.put(`/v1/ai-functions/${created.json.id}`, next)).status).toBe(200)
  })

  it("names are unique per workspace (422 validation)", async () => {
    const owner = await aiTenant("fn-unique")
    const other = await aiTenant("fn-unique-other")
    const input = functionInput()
    expect((await owner.api.post("/v1/ai-functions", input)).status).toBe(201)
    expectError(await owner.api.post("/v1/ai-functions", functionInput({ name: input.name })), 422, "validation")
    // Another workspace may use the same name.
    expect((await other.api.post("/v1/ai-functions", functionInput({ name: input.name }))).status).toBe(201)
    const second = await owner.api.post<AiFunction>("/v1/ai-functions", functionInput())
    expectError(
      await owner.api.put(`/v1/ai-functions/${second.json.id}`, functionInput({ name: input.name })),
      422,
      "validation",
    )
  })

  it("delete returns 204, then the function is gone; a missing id is 404", async () => {
    const { api } = await aiTenant("fn-delete")
    const created = await api.post<AiFunction>("/v1/ai-functions", functionInput())
    const deleted = await api.delete(`/v1/ai-functions/${created.json.id}`)
    expect(deleted.status).toBe(204)
    expect(deleted.text).toBe("")
    expectError(await api.get(`/v1/ai-functions/${created.json.id}`), 404, "notFound")
    expectError(await api.delete(`/v1/ai-functions/${created.json.id}`), 404, "notFound")
    expect((await api.get("/v1/ai-functions")).json).toEqual({ data: [], pageCount: 0 })
  })

  it("functions of another workspace are not visible", async () => {
    const owner = await aiTenant("fn-owner")
    const other = await aiTenant("fn-other")
    const created = await owner.api.post<AiFunction>("/v1/ai-functions", functionInput())
    const id = created.json.id
    expectError(await other.api.get(`/v1/ai-functions/${id}`), 404, "notFound")
    expectError(await other.api.put(`/v1/ai-functions/${id}`, functionInput()), 404, "notFound")
    expectError(await other.api.delete(`/v1/ai-functions/${id}`), 404, "notFound")
    expect((await owner.api.get(`/v1/ai-functions/${id}`)).status).toBe(200)
  })

  it("rejects invalid input with 422 invalidRequestData", async () => {
    const { api } = await aiTenant("fn-invalid")
    expectError(await api.post("/v1/ai-functions", functionInput({ name: " " })), 422, "invalidRequestData")
    expectError(await api.post("/v1/ai-functions", { name: "no_data_collect" }), 422, "invalidRequestData")
    expectError(
      await api.post("/v1/ai-functions", functionInput({ dataCollect: [{ from: "", to: "1" }] })),
      422,
      "invalidRequestData",
    )
    expectError(
      await api.post("/v1/ai-functions", functionInput({ triggerFlowId: "flow-1" })),
      422,
      "invalidRequestData",
    )
    expectError(await api.get("/v1/ai-functions/abc"), 422, "invalidRequestData")
    expectError(await api.get(`/v1/ai-functions/${MISSING_ID}`), 404, "notFound")
    expectError(await api.put(`/v1/ai-functions/${MISSING_ID}`, functionInput()), 404, "notFound")
    expectError(await api.put(`/v1/ai-functions/${MISSING_ID}`, { purpose: "x" }), 422, "invalidRequestData")
  })

  it("auth: 401 without a token, 403 without the automation scope, 403 for read-only writes", async () => {
    const owner = await aiTenant("fn-auth")
    const created = await owner.api.post<AiFunction>("/v1/ai-functions", functionInput())
    const id = created.json.id
    const none = apiClient("")
    const { wrongScope, readOnly } = await restrictedClients()
    const calls = [
      (api: typeof none) => api.get("/v1/ai-functions"),
      (api: typeof none) => api.get(`/v1/ai-functions/${id}`),
      (api: typeof none) => api.post("/v1/ai-functions", functionInput()),
      (api: typeof none) => api.put(`/v1/ai-functions/${id}`, functionInput()),
      (api: typeof none) => api.delete(`/v1/ai-functions/${id}`),
    ]
    for (const call of calls) {
      expectError(await call(none), 401, "INVALID_CHATBOT_TOKEN")
      expectError(await call(wrongScope), 403, "FORBIDDEN", AUTOMATION_SCOPE_MESSAGE)
    }
    expect((await readOnly.get("/v1/ai-functions")).json).toEqual({ data: [], pageCount: 0 })
    expectError(await readOnly.get(`/v1/ai-functions/${MISSING_ID}`), 404, "notFound")
    expectError(await readOnly.post("/v1/ai-functions", functionInput()), 403, "FORBIDDEN", READ_ONLY_MESSAGE)
    expectError(await readOnly.put(`/v1/ai-functions/${MISSING_ID}`, functionInput()), 403, "FORBIDDEN", READ_ONLY_MESSAGE)
    expectError(await readOnly.delete(`/v1/ai-functions/${MISSING_ID}`), 403, "FORBIDDEN", READ_ONLY_MESSAGE)
    expect((await owner.api.get(`/v1/ai-functions/${id}`)).status).toBe(200)
  })
})
