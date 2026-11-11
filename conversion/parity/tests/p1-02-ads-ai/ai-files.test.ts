// P1.2 — aiFiles.* (scope `automation`). Spec: conversion/specs/P1.2.md.
//
// Embeddings come from OpenAI, connected through the public API
// (`PUT /v1/integrations/ai/openai`). The worker processes an upload
// (extract, chunk) and embeds each chunk with `POST api.openai.com/v1/embeddings`,
// which the fake upstream answers per test (matched on a unique marker in the
// chunk text).

import { createHash } from "node:crypto"
import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { fake } from "../../src/fake.ts"
import { apiClient } from "../../src/http.ts"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueHost, uniqueId } from "../../src/unique.ts"
import {
  type AiFile,
  AUTOMATION_SCOPE_MESSAGE,
  aiTenant,
  alternatingVector,
  connectOpenAI,
  embeddingRequests,
  expectError,
  flatVector,
  MISSING_ID,
  markerText,
  READ_ONLY_MESSAGE,
  restrictedClients,
  scriptEmbeddings,
  uploadTextFile,
  waitForEmbedded,
} from "./ai-helpers.ts"

const DEFAULT_CHUNKING_CONFIG = {
  strategy: "auto",
  chunkSize: 1000,
  overlapSize: 200,
  tabularHeaderRow: 1,
  isDefault: true,
}

/** AIEmbedding id: first 60 bits of SHA-256 over {aiFileId, chunkIndex, content}. */
const expectedChunkId = (aiFileId: string, chunkIndex: number, content: string): string => {
  const digest = createHash("sha256").update(JSON.stringify({ aiFileId, chunkIndex, content })).digest("hex")
  const id = BigInt(`0x${digest.slice(0, 15)}`)
  return id === 0n ? "1" : id.toString()
}

type Chunk = { id: string; chunkIndex: number | null; content: string; status: string; createdAt: string }
type ChunkPage = { data: Chunk[]; nextCursor: string | null }

/** Tenant with OpenAI connected and an embedding script for `marker`. */
const readyTenant = async (label: string) => {
  const tenant = await aiTenant(label)
  const apiKey = await connectOpenAI(tenant.api)
  const marker = `kb${uniqueId()}`
  await scriptEmbeddings(marker, flatVector())
  return { ...tenant, apiKey, marker }
}

/** Uploads a one-chunk text file and waits until it is embedded. */
const embeddedFile = async (tenant: Awaited<ReturnType<typeof readyTenant>>, name?: string) => {
  const text = markerText(tenant.marker, 3)
  const created = await uploadTextFile(tenant.token, { text, name, fileName: "faq.txt" })
  expect(created.status, created.text).toBe(201)
  const file = await waitForEmbedded(tenant.api, created.json.id, 1)
  return { text, file }
}

describe("P1.2 aiFiles", () => {
  it("create (multipart) returns 201 with a queued run; the worker chunks and embeds the file", async () => {
    const normalizer = new Normalizer()
    const tenant = await readyTenant("file-create")
    const text = markerText(tenant.marker, 3)
    const created = await uploadTextFile(tenant.token, { text, name: "Parity FAQ", fileName: "faq.txt" })
    expect(created.status, created.text).toBe(201)
    const { url, path, ...rest } = created.json
    expect(path).toBe(`workspaces/${tenant.workspaceId}/ai-files/${path.split("/").at(-1)}`)
    expect(path.split("/").at(-1)).toMatch(/^\d{15,20}$/)
    expect(normalizer.normalize(rest, { replace: { [tenant.workspaceId]: "<ws>" } })).toEqual({
      id: "<id:1>",
      workspaceId: "<ws>",
      createdAt: "<ts>",
      updatedAt: "<ts>",
      name: "Parity FAQ",
      size: Buffer.byteLength(text),
      mimeType: "text/plain",
      chunking: null,
      chunkingConfig: DEFAULT_CHUNKING_CONFIG,
      chunkingOutdated: false,
      indexOutdated: false,
      chunksCount: 0,
      processingStatus: "pending",
      processingRun: {
        id: "<id:2>",
        status: "queued",
        error: null,
        queuedAt: "<ts>",
        startedAt: null,
        heartbeatAt: null,
        finishedAt: null,
        stale: false,
      },
    })
    // `url` is a time-limited download link to the stored bytes.
    const download = await fetch(url)
    expect(download.status).toBe(200)
    expect(await download.text()).toBe(text)

    const done = await waitForEmbedded(tenant.api, created.json.id, 1)
    const { url: _url, ...doneRest } = done
    expect(normalizer.normalize(doneRest, { replace: { [tenant.workspaceId]: "<ws>" } })).toMatchObject({
      id: "<id:1>",
      chunksCount: 1,
      processingStatus: "success",
      chunkingOutdated: false,
      indexOutdated: false,
      chunking: {
        strategy: "prose",
        chunkerVersion: 1,
        chunkSize: 1000,
        overlapSize: 200,
        chunkCount: 1,
        processedAt: "<ts>",
        extractedChars: text.length,
      },
      processingRun: {
        id: "<id:2>",
        status: "success",
        error: null,
        queuedAt: "<ts>",
        startedAt: "<ts>",
        heartbeatAt: "<ts>",
        finishedAt: "<ts>",
        stale: false,
      },
    })

    // One embedding call per chunk, with the workspace's OpenAI key.
    const calls = await embeddingRequests(tenant.marker)
    expect(calls).toHaveLength(1)
    expect(calls[0]?.headers.authorization).toBe(`Bearer ${tenant.apiKey}`)
    const sent = JSON.parse(calls[0]?.bodyText ?? "{}") as { model?: string; input?: unknown }
    expect(sent.model).toBe("text-embedding-ada-002")
    expect(sent.input).toEqual([text])
  })

  it("create without a name uses the uploaded file name", async () => {
    const tenant = await readyTenant("file-noname")
    const created = await uploadTextFile(tenant.token, { text: markerText(tenant.marker, 2), fileName: "notes.txt" })
    expect(created.status, created.text).toBe(201)
    expect(created.json.name).toBe("notes.txt")
    await waitForEmbedded(tenant.api, created.json.id, 1)
  })

  it("create from a URL downloads the file through the SSRF guard", async () => {
    const tenant = await readyTenant("file-url")
    const host = uniqueHost("kb")
    const text = markerText(tenant.marker, 2)
    await fake.script({
      match: { host, method: "GET", path: "/docs/handbook.txt" },
      respond: { status: 200, headers: { "content-type": "text/plain; charset=utf-8" }, text },
    })
    const created = await tenant.api.post<AiFile>("/v1/ai-files", { url: `https://${host}/docs/handbook.txt` })
    expect(created.status, created.text).toBe(201)
    expect(created.json).toMatchObject({
      name: "handbook.txt",
      mimeType: "text/plain",
      size: Buffer.byteLength(text),
      processingStatus: "pending",
      processingRun: { status: "queued" },
    })
    const downloads = await fake.requests({ kind: "http", host })
    expect(downloads).toHaveLength(1)
    expect(downloads[0]).toMatchObject({ scheme: "https", method: "GET", path: "/docs/handbook.txt" })
    const doh = await fake.requests({ kind: "dns", host })
    expect(doh.some((record) => record.dnsTransport === "doh")).toBe(true)
    await waitForEmbedded(tenant.api, created.json.id, 1)
  })

  it("create rejects a URL that resolves to a private address and a failed download (400 businessError)", async () => {
    const tenant = await readyTenant("file-url-bad")
    const privateHost = uniqueHost("kb-private")
    await fake.dns({ name: privateHost, a: ["10.0.0.9"] })
    const blocked = await tenant.api.post("/v1/ai-files", { url: `https://${privateHost}/a.txt` })
    expectError(blocked, 400, "businessError", "The provided URL is not allowed")
    expect(await fake.requests({ kind: "http", host: privateHost })).toHaveLength(0)

    const missingHost = uniqueHost("kb-missing")
    await fake.script({ match: { host: missingHost, method: "GET", path: "/a.txt" }, respond: { status: 404, text: "no" } })
    const failed = await tenant.api.post("/v1/ai-files", { url: `https://${missingHost}/a.txt` })
    expectError(failed, 400, "businessError", "Failed to download file: 404")
    expect((await tenant.api.get("/v1/ai-files")).json).toEqual({ data: [], pageCount: 0 })
  })

  it("create needs an OpenAI or Gemini integration (400 noEmbeddingProvider)", async () => {
    const tenant = await aiTenant("file-noprovider")
    const response = await uploadTextFile(tenant.token, { text: "hello" })
    expectError(response, 400, "noEmbeddingProvider")
    expect((await tenant.api.get("/v1/ai-files")).json).toEqual({ data: [], pageCount: 0 })
  })

  it("create rejects both or neither of file and url (422 invalidRequestData)", async () => {
    const tenant = await readyTenant("file-invalid")
    expectError(await tenant.api.post("/v1/ai-files", { name: "x" }), 422, "invalidRequestData")
    expectError(
      await uploadTextFile(tenant.token, { text: "hello", extraFields: { url: "https://example.parity.test/a.txt" } }),
      422,
      "invalidRequestData",
    )
    expectError(await tenant.api.post("/v1/ai-files", { url: "not a url" }), 422, "invalidRequestData")
  })

  it("list pages newest first and get returns one file; another workspace sees nothing", async () => {
    const tenant = await readyTenant("file-list")
    expect((await tenant.api.get("/v1/ai-files")).json).toEqual({ data: [], pageCount: 0 })
    const first = await uploadTextFile(tenant.token, { text: markerText(tenant.marker, 1), name: "first" })
    const second = await uploadTextFile(tenant.token, { text: markerText(tenant.marker, 2), name: "second" })
    expect(first.status).toBe(201)
    expect(second.status).toBe(201)
    const listed = await tenant.api.get<{ data: AiFile[]; pageCount: number }>("/v1/ai-files")
    expect(listed.status).toBe(200)
    expect(listed.json.pageCount).toBe(1)
    expect(listed.json.data.map((file) => file.name)).toEqual(["second", "first"])
    const page2 = await tenant.api.get<{ data: AiFile[]; pageCount: number }>("/v1/ai-files", {
      query: { page: 2, perPage: 1 },
    })
    expect(page2.json.pageCount).toBe(2)
    expect(page2.json.data.map((file) => file.name)).toEqual(["first"])

    const got = await tenant.api.get<AiFile>(`/v1/ai-files/${first.json.id}`)
    expect(got.status).toBe(200)
    expect(got.json).toMatchObject({ id: first.json.id, name: "first", workspaceId: tenant.workspaceId })

    const other = await aiTenant("file-list-other")
    expectError(await other.api.get(`/v1/ai-files/${first.json.id}`), 404, "notFound")
    expectError(await other.api.get(`/v1/ai-files/${first.json.id}/chunks`), 404, "notFound")
    expectError(await tenant.api.get(`/v1/ai-files/${MISSING_ID}`), 404, "notFound")
    expectError(await tenant.api.get("/v1/ai-files/xyz"), 422, "invalidRequestData")
    await waitForEmbedded(tenant.api, first.json.id, 1)
    await waitForEmbedded(tenant.api, second.json.id, 1)
  })

  it("listChunks returns the chunks in order with an opaque cursor", async () => {
    const normalizer = new Normalizer()
    const tenant = await readyTenant("file-chunks")
    const { text, file } = await embeddedFile(tenant)
    const page = await tenant.api.get<ChunkPage>(`/v1/ai-files/${file.id}/chunks`)
    expect(page.status).toBe(200)
    // Chunk ids are derived from (file id, position, content).
    expect(normalizer.normalize(page.json, { keep: ["id"] })).toEqual({
      data: [{ id: expectedChunkId(file.id, 0, text), chunkIndex: 0, content: text, status: "success", createdAt: "<ts>" }],
      nextCursor: null,
    })
    const skipped = await tenant.api.get<ChunkPage>(`/v1/ai-files/${file.id}/chunks`, { query: { fromChunkIndex: 1 } })
    expect(skipped.json).toEqual({ data: [], nextCursor: null })

    expectError(
      await tenant.api.get(`/v1/ai-files/${file.id}/chunks`, { query: { cursor: "not-a-cursor" } }),
      400,
      "invalidCursor",
      "The cursor is invalid. Start again without a cursor.",
    )
    expectError(await tenant.api.get(`/v1/ai-files/${file.id}/chunks`, { query: { limit: 101 } }), 422, "invalidRequestData")
    expectError(await tenant.api.get(`/v1/ai-files/${file.id}/chunks`, { query: { limit: 0 } }), 422, "invalidRequestData")
    expectError(await tenant.api.get(`/v1/ai-files/${MISSING_ID}/chunks`), 404, "notFound")
  })

  it("reindex with new chunking parameters re-chunks and re-embeds the file", async () => {
    const tenant = await readyTenant("file-reindex")
    const text = markerText(tenant.marker, 15) // 344 characters
    const created = await uploadTextFile(tenant.token, { text })
    expect(created.status).toBe(201)
    const before = await waitForEmbedded(tenant.api, created.json.id, 1)

    const reindexed = await tenant.api.post<AiFile & { coalesced: boolean }>(
      `/v1/ai-files/${created.json.id}/reindex`,
      { chunkingConfig: { strategy: "prose", chunkSize: 200, overlapSize: 0 } },
    )
    expect(reindexed.status, reindexed.text).toBe(202)
    expect(reindexed.json).not.toHaveProperty("workspaceId")
    expect(reindexed.json).toMatchObject({
      id: created.json.id,
      coalesced: false,
      chunkingConfig: { strategy: "prose", chunkSize: 200, overlapSize: 0, tabularHeaderRow: 1, isDefault: false },
      processingRun: { status: "queued", error: null, startedAt: null, finishedAt: null, stale: false },
    })
    expect(reindexed.json.processingRun?.id).not.toBe(before.processingRun?.id)

    const after = await waitForEmbedded(tenant.api, created.json.id, 2)
    expect(after.chunking).toMatchObject({ strategy: "prose", chunkSize: 200, overlapSize: 0, chunkCount: 2 })
    expect(after.processingRun).toMatchObject({ id: reindexed.json.processingRun?.id, status: "success" })

    // Two chunks: page through them one at a time.
    const first = await tenant.api.get<ChunkPage>(`/v1/ai-files/${created.json.id}/chunks`, { query: { limit: 1 } })
    expect(first.json.data.map((chunk) => [chunk.chunkIndex, chunk.content])).toEqual([[0, text.slice(0, 200).trim()]])
    expect(typeof first.json.nextCursor).toBe("string")
    const second = await tenant.api.get<ChunkPage>(`/v1/ai-files/${created.json.id}/chunks`, {
      query: { limit: 1, cursor: first.json.nextCursor ?? "" },
    })
    expect(second.json.data.map((chunk) => [chunk.chunkIndex, chunk.content])).toEqual([[1, text.slice(200).trim()]])
    expect(second.json.nextCursor).toBeNull()
    const from1 = await tenant.api.get<ChunkPage>(`/v1/ai-files/${created.json.id}/chunks`, { query: { fromChunkIndex: 1 } })
    expect(from1.json.data.map((chunk) => chunk.chunkIndex)).toEqual([1])

    // 1 embedding for the first chunk, then 2 for the new chunks.
    expect(await embeddingRequests(tenant.marker)).toHaveLength(3)

    // Reindex without parameters keeps them; null resets to the defaults.
    const plain = await tenant.api.post<AiFile & { coalesced: boolean }>(`/v1/ai-files/${created.json.id}/reindex`, {})
    expect(plain.status).toBe(202)
    expect(plain.json.chunkingConfig).toMatchObject({ chunkSize: 200, isDefault: false })
    await waitForEmbedded(tenant.api, created.json.id, 2)
    const reset = await tenant.api.post<AiFile>(`/v1/ai-files/${created.json.id}/reindex`, { chunkingConfig: null })
    expect(reset.status).toBe(202)
    expect(reset.json.chunkingConfig).toEqual(DEFAULT_CHUNKING_CONFIG)
    await waitForEmbedded(tenant.api, created.json.id, 1)
  })

  it("reindex errors: 404 unknown, 422 bad parameters, 400 without an embedding provider", async () => {
    const tenant = await readyTenant("file-reindex-bad")
    const { file } = await embeddedFile(tenant)
    expectError(await tenant.api.post(`/v1/ai-files/${MISSING_ID}/reindex`, {}), 404, "notFound")
    expectError(
      await tenant.api.post(`/v1/ai-files/${file.id}/reindex`, { chunkingConfig: { chunkSize: 100 } }),
      422,
      "invalidRequestData",
    )
    expectError(
      await tenant.api.post(`/v1/ai-files/${file.id}/reindex`, { chunkingConfig: { strategy: "tabular" } }),
      422,
      "invalidRequestData",
    )
    expectError(
      await tenant.api.post(`/v1/ai-files/${file.id}/reindex`, { chunkingConfig: { unknown: 1 } }),
      422,
      "invalidRequestData",
    )
    const disconnected = await tenant.api.delete("/v1/integrations/ai/openai")
    expect(disconnected.status).toBe(204)
    expectError(await tenant.api.post(`/v1/ai-files/${file.id}/reindex`, {}), 400, "noEmbeddingProvider")
  })

  it("search by fileIds embeds the query and returns the matching chunk", async () => {
    const normalizer = new Normalizer()
    const tenant = await readyTenant("file-search")
    const { text, file } = await embeddedFile(tenant, "Handbook")
    const query = `where is ${tenant.marker}`
    const searched = await tenant.api.post("/v1/ai-files/search", { query, fileIds: [file.id] })
    expect(searched.status, searched.text).toBe(200)
    const body = searched.json as {
      results: Array<{ similarity: number; chunkId: string }>
      timingsMs: { embed: number; search: number }
    }
    expect(body.results[0]?.similarity).toBeCloseTo(1, 5)
    expect(typeof body.timingsMs.embed).toBe("number")
    expect(typeof body.timingsMs.search).toBe("number")
    const stable = {
      ...(searched.json as object),
      results: body.results.map((result) => ({ ...result, similarity: "<~1>" })),
      timingsMs: "<timings>",
    }
    const chunkId = body.results[0]?.chunkId ?? ""
    expect(chunkId).toBe(expectedChunkId(file.id, 0, text))
    expect(normalizer.normalize(stable, { replace: { [file.id]: "<file>", [chunkId]: "<chunk>" } })).toEqual({
      mode: "files",
      results: [
        {
          chunkId: "<chunk>",
          fileId: "<file>",
          fileName: "Handbook",
          chunkIndex: 0,
          content: text,
          similarity: "<~1>",
          rank: 1,
          passedThreshold: true,
          usedByAgent: null,
          agentVisible: true,
        },
      ],
      appliedConfig: {
        provider: "openai",
        modelId: "text-embedding-ada-002",
        similarityThreshold: 0.7,
        agentDefaultThreshold: 0.7,
        agentMaxResults: 5,
        limit: 10,
      },
      files: [{ id: "<file>", name: "Handbook", chunksCount: 1, processingStatus: "success", readyChunks: 1 }],
      diagnosis: "ok",
      expected: null,
      timingsMs: "<timings>",
    })
    // The query embedding call: 1 for the chunk + 1 for the query.
    const calls = await embeddingRequests(tenant.marker)
    expect(calls).toHaveLength(2)
    expect(JSON.parse(calls[1]?.bodyText ?? "{}").input).toEqual([query])
  })

  it("search reports expectedText, a trial threshold and a weak result", async () => {
    const tenant = await readyTenant("file-search-diag")
    const { file } = await embeddedFile(tenant, "Diag")
    const found = await tenant.api.post<{ expected: unknown; appliedConfig: { limit: number; similarityThreshold: number } }>(
      "/v1/ai-files/search",
      { query: `find ${tenant.marker}`, fileIds: [file.id], expectedText: tenant.marker.toUpperCase(), limit: 1, similarityThreshold: 0.2 },
    )
    expect(found.status, found.text).toBe(200)
    expect(found.json.appliedConfig).toMatchObject({ limit: 5, similarityThreshold: 0.2 })
    const chunkId = expectedChunkId(file.id, 0, markerText(tenant.marker, 3))
    expect(new Normalizer().normalize(found.json.expected, { replace: { [file.id]: "<file>", [chunkId]: "<chunk>" } })).toEqual({
      matches: [{ fileId: "<file>", fileName: "Diag", chunkId: "<chunk>", chunkIndex: 0, rank: 1 }],
      verdict: "inTopResults",
    })
    const missing = await tenant.api.post<{ expected: unknown }>("/v1/ai-files/search", {
      query: `find ${tenant.marker}`,
      fileIds: [file.id],
      expectedText: "not in the file",
    })
    expect(missing.json.expected).toEqual({ matches: [], verdict: "notInAnyChunk" })

    // An orthogonal query vector: the chunk is found but below the threshold.
    const weakMarker = `weak${uniqueId()}`
    await scriptEmbeddings(weakMarker, alternatingVector())
    const weak = await tenant.api.post<{
      diagnosis: string
      results: Array<{ passedThreshold: boolean; agentVisible: boolean; similarity: number }>
    }>("/v1/ai-files/search", { query: weakMarker, fileIds: [file.id] })
    expect(weak.status).toBe(200)
    expect(weak.json.diagnosis).toBe("allBelowThreshold")
    expect(weak.json.results).toHaveLength(1)
    expect(weak.json.results[0]).toMatchObject({ passedThreshold: false, agentVisible: false })
    expect(weak.json.results[0]?.similarity).toBeCloseTo(0, 5)
  })

  it("search by agentId uses the agent's file tools", async () => {
    const tenant = await readyTenant("file-search-agent")
    const { file } = await embeddedFile(tenant, "Agent KB")
    const agentInput = {
      name: `KB agent ${uniqueId()}`,
      prompt: "Answer from the knowledge base.",
      messages: [],
      models: [{ provider: "openai", model: "gpt-4o-mini" }],
      temperature: 0.2,
      maxOutputTokens: 300,
      isDefault: false,
    }
    const agent = await tenant.api.post<{ id: string }>("/v1/ai-agents", { ...agentInput, tools: [`file:${file.id}`] })
    expect(agent.status, agent.text).toBe(201)
    const searched = await tenant.api.post<{
      mode: string
      results: Array<{ usedByAgent: boolean | null; agentVisible: boolean; fileId: string }>
      appliedConfig: { limit: number }
      diagnosis: string
    }>("/v1/ai-files/search", { query: tenant.marker, agentId: agent.json.id, limit: 2 })
    expect(searched.status, searched.text).toBe(200)
    expect(searched.json).toMatchObject({ mode: "agent", diagnosis: "ok", appliedConfig: { limit: 5 } })
    expect(searched.json.results).toEqual([
      expect.objectContaining({ fileId: file.id, usedByAgent: true, agentVisible: true }),
    ])

    // An agent without file tools: nothing is embedded.
    const before = (await embeddingRequests(tenant.marker)).length
    const bare = await tenant.api.post<{ id: string }>("/v1/ai-agents", { ...agentInput, tools: [] })
    const empty = await tenant.api.post("/v1/ai-files/search", { query: tenant.marker, agentId: bare.json.id })
    expect(empty.status).toBe(200)
    expect(empty.json).toEqual({
      mode: "agent",
      results: [],
      appliedConfig: null,
      files: [],
      diagnosis: "agentHasNoFiles",
      expected: null,
      timingsMs: { embed: 0, search: 0 },
    })
    expect(await embeddingRequests(tenant.marker)).toHaveLength(before)
  })

  it("search errors: 422 for bad input, 404 for unknown files or agents", async () => {
    const tenant = await readyTenant("file-search-bad")
    const created = await uploadTextFile(tenant.token, { text: markerText(tenant.marker, 1) })
    const fileId = created.json.id
    expectError(await tenant.api.post("/v1/ai-files/search", { query: "x" }), 422, "invalidRequestData")
    expectError(
      await tenant.api.post("/v1/ai-files/search", { query: "x", fileIds: [fileId], agentId: MISSING_ID }),
      422,
      "invalidRequestData",
    )
    expectError(await tenant.api.post("/v1/ai-files/search", { query: " ", fileIds: [fileId] }), 422, "invalidRequestData")
    expectError(
      await tenant.api.post("/v1/ai-files/search", { query: "x".repeat(501), fileIds: [fileId] }),
      422,
      "invalidRequestData",
    )
    expectError(await tenant.api.post("/v1/ai-files/search", { query: "x", fileIds: [] }), 422, "invalidRequestData")
    expectError(
      await tenant.api.post("/v1/ai-files/search", { query: "x", fileIds: [fileId], limit: 21 }),
      422,
      "invalidRequestData",
    )
    expectError(
      await tenant.api.post("/v1/ai-files/search", { query: "x", fileIds: [fileId, MISSING_ID] }),
      404,
      "notFound",
    )
    expectError(await tenant.api.post("/v1/ai-files/search", { query: "x", agentId: MISSING_ID }), 404, "notFound")
    await waitForEmbedded(tenant.api, fileId, 1)
  })

  it("delete returns 204 and removes the file and its stored bytes", async () => {
    const tenant = await readyTenant("file-delete")
    const { file } = await embeddedFile(tenant)
    const deleted = await tenant.api.delete(`/v1/ai-files/${file.id}`)
    expect(deleted.status).toBe(204)
    expect(deleted.text).toBe("")
    expectError(await tenant.api.get(`/v1/ai-files/${file.id}`), 404, "notFound")
    expectError(await tenant.api.get(`/v1/ai-files/${file.id}/chunks`), 404, "notFound")
    expect((await tenant.api.get("/v1/ai-files")).json).toEqual({ data: [], pageCount: 0 })
    await eventually(async () => expect((await fetch(file.url)).status).toBe(404))
    expectError(await tenant.api.delete(`/v1/ai-files/${file.id}`), 404, "notFound")
    const other = await aiTenant("file-delete-other")
    expectError(await other.api.delete(`/v1/ai-files/${MISSING_ID}`), 404, "notFound")
  })

  it("auth: 401 without a token, 403 without the automation scope, read-only may only read", async () => {
    const tenant = await readyTenant("file-auth")
    const { file } = await embeddedFile(tenant)
    const id = file.id
    const none = apiClient("")
    const { wrongScope, readOnly, readOnlyTenant } = await restrictedClients()
    const calls = [
      (api: typeof none) => api.get("/v1/ai-files"),
      (api: typeof none) => api.get(`/v1/ai-files/${id}`),
      (api: typeof none) => api.post("/v1/ai-files", { url: "https://example.parity.test/a.txt" }),
      (api: typeof none) => api.post("/v1/ai-files/search", { query: "x", fileIds: [id] }),
      (api: typeof none) => api.get(`/v1/ai-files/${id}/chunks`),
      (api: typeof none) => api.post(`/v1/ai-files/${id}/reindex`, {}),
      (api: typeof none) => api.delete(`/v1/ai-files/${id}`),
    ]
    for (const call of calls) {
      expectError(await call(none), 401, "INVALID_CHATBOT_TOKEN")
      expectError(await call(wrongScope), 403, "FORBIDDEN", AUTOMATION_SCOPE_MESSAGE)
    }
    expectError(await uploadTextFile("", { text: "x" }), 401, "INVALID_CHATBOT_TOKEN")

    // A read-only token reads (list, get, listChunks) but cannot write or search.
    expect((await readOnly.get("/v1/ai-files")).json).toEqual({ data: [], pageCount: 0 })
    expectError(await readOnly.get(`/v1/ai-files/${id}`), 404, "notFound")
    expectError(await readOnly.get(`/v1/ai-files/${id}/chunks`), 404, "notFound")
    expectError(await uploadTextFile(readOnlyTenant.token, { text: "x" }), 403, "FORBIDDEN", READ_ONLY_MESSAGE)
    expectError(
      await readOnly.post("/v1/ai-files/search", { query: "x", fileIds: [MISSING_ID] }),
      403,
      "FORBIDDEN",
      READ_ONLY_MESSAGE,
    )
    expectError(await readOnly.post(`/v1/ai-files/${MISSING_ID}/reindex`, {}), 403, "FORBIDDEN", READ_ONLY_MESSAGE)
    expectError(await readOnly.delete(`/v1/ai-files/${MISSING_ID}`), 403, "FORBIDDEN", READ_ONLY_MESSAGE)
    expect((await tenant.api.get(`/v1/ai-files/${id}`)).status).toBe(200)
  })
})
