// AI provider integrations: integrations.getAiProvider (GET), connectAiProvider
// (deprecated PUT, upsert), updateAiProvider (PATCH autoReply) and
// disconnectAiProvider (deprecated DELETE) on /v1/integrations/ai/{provider}.
// connect checks the key live with the provider's "list models" endpoint:
// Claude `GET https://api.anthropic.com/v1/models` (x-api-key), Gemini
// `GET https://generativelanguage.googleapis.com/v1beta/models` (x-goog-api-key).
// Only this file scripts those two hosts; every scripted answer is consumed.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  type Api,
  expectError,
  expectInvalid,
  expectNotFound,
  newTenant,
  SNOWFLAKE,
  upstreamCalls,
} from "./helpers.ts"

const CLAUDE = { host: "api.anthropic.com", path: "/v1/models" }
const GEMINI = { host: "generativelanguage.googleapis.com", path: "/v1beta/models" }

type Provider = {
  id: string
  model: string
  temperature: number | null
  maxOutputTokens: number
  autoReply: boolean
  hasApiKey: boolean
}

/** Answers the next key check of `target` with `status` (consumed by exactly one connect). */
const answerKeyCheck = (target: { host: string; path: string }, status: number) =>
  fake.script({
    match: { host: target.host, method: "GET", path: target.path },
    respond: status === 200 ? { status, json: { data: [], has_more: false } } : { status, json: { error: { message: "x" } } },
    times: 1,
  })

const connectClaude = async (api: Api, body: Record<string, unknown> = {}) => {
  await answerKeyCheck(CLAUDE, 200)
  const response = await api.put<Provider>("/v1/integrations/ai/claude", {
    apiKey: `sk-ant-${uniqueId()}`,
    model: "claude-3-5-haiku",
    temperature: 0.4,
    maxOutputTokens: 1024,
    ...body,
  })
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("integrations.connectAiProvider", () => {
  it("checks the key with the provider, connects it and never returns the key", async () => {
    const { api } = await newTenant("p110-ai-connect")
    const apiKey = `sk-ant-${uniqueId()}`
    await answerKeyCheck(CLAUDE, 200)
    const since = await fake.cursor()
    const response = await api.put<Provider>("/v1/integrations/ai/claude", {
      apiKey,
      model: "claude-3-5-haiku",
      temperature: "0.4",
      maxOutputTokens: "1024",
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      model: "claude-3-5-haiku",
      temperature: 0.4,
      maxOutputTokens: 1024,
      autoReply: false,
      hasApiKey: true,
    })
    expect(response.text).not.toContain(apiKey)
    const calls = await upstreamCalls({ host: CLAUDE.host, since })
    expect(calls).toHaveLength(1)
    expect(calls[0]).toMatchObject({ method: "GET", path: CLAUDE.path })
    expect(calls[0]?.headers["x-api-key"]).toBe(apiKey)
    expect(calls[0]?.headers["anthropic-version"]).toBe("2023-06-01")

    const read = await api.get<Provider>("/v1/integrations/ai/claude")
    expect(read.status, read.text).toBe(200)
    expect(read.json).toEqual(response.json)
    const listed = await api.get<{ data: Array<{ integrationType: string }> }>("/v1/integrations")
    expect(listed.json.data.map((item) => item.integrationType)).toEqual(["claude"])
  })

  it("a second PUT replaces model and key in place (same id)", async () => {
    const { api } = await newTenant("p110-ai-upsert")
    const first = await connectClaude(api)
    const since = await fake.cursor()
    const second = await connectClaude(api, { apiKey: "sk-ant-second", model: "claude-sonnet", temperature: 1.5, maxOutputTokens: 8192 })
    expect(second).toEqual({ ...first, model: "claude-sonnet", temperature: 1.5, maxOutputTokens: 8192 })
    const calls = await upstreamCalls({ host: CLAUDE.host, since })
    expect(calls.map((call) => call.headers["x-api-key"])).toEqual(["sk-ant-second"])
  })

  it("providers are separate (Gemini sends its key as x-goog-api-key)", async () => {
    const { api } = await newTenant("p110-ai-gemini")
    await answerKeyCheck(GEMINI, 200)
    const since = await fake.cursor()
    const response = await api.put<Provider>("/v1/integrations/ai/gemini", {
      apiKey: "gem-key",
      model: "gemini-2.0-flash",
      temperature: 0,
      maxOutputTokens: 1,
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ model: "gemini-2.0-flash", temperature: 0, maxOutputTokens: 1, autoReply: false })
    expect((await upstreamCalls({ host: GEMINI.host, since }))[0]?.headers["x-goog-api-key"]).toBe("gem-key")
    expectNotFound(await api.get("/v1/integrations/ai/claude"), "claude integration not found")
  })

  it("a rejected key (401) is 400 connectionCredentialsRejected and connects nothing", async () => {
    const { api } = await newTenant("p110-ai-reject")
    await answerKeyCheck(CLAUDE, 401)
    expectError(
      await api.put("/v1/integrations/ai/claude", { apiKey: "sk-bad", model: "m", temperature: 1, maxOutputTokens: 10 }),
      400,
      "connectionCredentialsRejected",
    )
    expectNotFound(await api.get("/v1/integrations/ai/claude"), "claude integration not found")
  })

  it("an inconclusive key check (upstream 500) is 502 connectionProviderUnavailable", async () => {
    const { api } = await newTenant("p110-ai-unavail")
    await answerKeyCheck(CLAUDE, 500)
    expectError(
      await api.put("/v1/integrations/ai/claude", { apiKey: "sk-x", model: "m", temperature: 1, maxOutputTokens: 10 }),
      502,
      "connectionProviderUnavailable",
      { message: "The provider is temporarily unavailable. Please try again." },
    )
    expectNotFound(await api.get("/v1/integrations/ai/claude"), "claude integration not found")
  })

  it("validates the input before calling the provider (422 invalidRequestData)", async () => {
    const { api } = await newTenant("p110-ai-invalid")
    const since = await fake.cursor()
    const base = { apiKey: "k", model: "m", temperature: 1, maxOutputTokens: 10 }
    expectInvalid(await api.put("/v1/integrations/ai/openrouter", base), "provider")
    expectInvalid(await api.put("/v1/integrations/ai/claude", { ...base, apiKey: "" }), "apiKey")
    expectInvalid(await api.put("/v1/integrations/ai/claude", { ...base, model: "" }), "model")
    expectInvalid(await api.put("/v1/integrations/ai/claude", { ...base, temperature: 2.5 }), "temperature")
    expectInvalid(await api.put("/v1/integrations/ai/claude", { ...base, maxOutputTokens: 0 }), "maxOutputTokens")
    expectInvalid(await api.put("/v1/integrations/ai/claude", { ...base, maxOutputTokens: 8193 }), "maxOutputTokens")
    expectInvalid(await api.put("/v1/integrations/ai/claude", { ...base, maxOutputTokens: 1.5 }), "maxOutputTokens")
    expectInvalid(await api.put("/v1/integrations/ai/claude", { model: "m", temperature: 1, maxOutputTokens: 10 }), "apiKey")
    expect(await upstreamCalls({ host: CLAUDE.host, since })).toEqual([])
  })
})

describe("integrations.getAiProvider", () => {
  it("is 404 before connect; 422 for an unknown provider", async () => {
    const { api } = await newTenant("p110-ai-get")
    expectNotFound(await api.get("/v1/integrations/ai/claude"), "claude integration not found")
    expectNotFound(await api.get("/v1/integrations/ai/muse"), "muse integration not found")
    expectInvalid(await api.get("/v1/integrations/ai/gpt"), "provider")
  })

  it("does not see another workspace's provider", async () => {
    const { api } = await newTenant("p110-ai-get-iso")
    await connectClaude(api)
    const { api: otherApi } = await newTenant("p110-ai-get-iso-o")
    expectNotFound(await otherApi.get("/v1/integrations/ai/claude"), "claude integration not found")
  })
})

describe("integrations.updateAiProvider", () => {
  it("switches autoReply on and off; an empty body changes nothing", async () => {
    const { api } = await newTenant("p110-ai-patch")
    const connected = await connectClaude(api)
    const on = await api.patch<Provider>("/v1/integrations/ai/claude", { autoReply: true })
    expect(on.status, on.text).toBe(200)
    expect(on.json).toEqual({ ...connected, autoReply: true })
    expect((await api.get<Provider>("/v1/integrations/ai/claude")).json.autoReply).toBe(true)
    const same = await api.patch<Provider>("/v1/integrations/ai/claude", {})
    expect(same.json).toEqual({ ...connected, autoReply: true })
    const off = await api.patch<Provider>("/v1/integrations/ai/claude", { autoReply: false })
    expect(off.json).toEqual(connected)
  })

  it("is 404 before connect; 422 for bad input", async () => {
    const { api } = await newTenant("p110-ai-patch-404")
    expectNotFound(await api.patch("/v1/integrations/ai/claude", { autoReply: true }), "claude integration not found")
    expectInvalid(await api.patch("/v1/integrations/ai/claude", { autoReply: "yes" }), "autoReply")
    // oRPC coerces the strings "on" / "off" to booleans: this passes validation and reaches the 404.
    expectNotFound(await api.patch("/v1/integrations/ai/claude", { autoReply: "on" }), "claude integration not found")
    expectInvalid(await api.patch("/v1/integrations/ai/openaiCompatible", { autoReply: true }), "provider")
  })
})

describe("integrations.disconnectAiProvider", () => {
  it("disconnects (204); the provider is gone; a repeat is still 204; reconnect gives a new id", async () => {
    const { api } = await newTenant("p110-ai-delete")
    const first = await connectClaude(api)
    const since = await fake.cursor()
    const response = await api.delete("/v1/integrations/ai/claude")
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expect(await upstreamCalls({ host: CLAUDE.host, since })).toEqual([])
    expectNotFound(await api.get("/v1/integrations/ai/claude"), "claude integration not found")
    expect((await api.get("/v1/integrations")).json).toEqual({ data: [], pageCount: 1 })
    expect((await api.delete("/v1/integrations/ai/claude")).status).toBe(204)

    const again = await connectClaude(api)
    expect(again.id).not.toBe(first.id)
  })

  it("never connected: 204; unknown provider: 422", async () => {
    const { api } = await newTenant("p110-ai-delete-none")
    expect((await api.delete("/v1/integrations/ai/deepseek")).status).toBe(204)
    expectInvalid(await api.delete("/v1/integrations/ai/foo"), "provider")
  })
})
