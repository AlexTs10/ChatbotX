// Shared helpers of the P1.2 AI tests (aiAgents, aiFunctions, aiFiles).
// Not a test file (no `.test.ts`), so vitest does not run it.

import { expect } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { fake } from "../../src/fake.ts"
import { createTenant, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, HttpClient, type HttpResponse } from "../../src/http.ts"
import { comparableError } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"

export type Api = ReturnType<typeof apiClient>

/** A snowflake-shaped id that no row has. */
export const MISSING_ID = "999999999999999999"

export const AUTOMATION_SCOPE_MESSAGE = "Token is not authorized for the 'automation' scope"
export const READ_ONLY_MESSAGE = "Read-only token cannot perform this operation"

export const expectError = (
  response: HttpResponse,
  status: number,
  code: string,
  message?: string,
) => {
  expect(response.status, response.text).toBe(status)
  expect(comparableError(response.json, { keepMessage: message !== undefined })).toMatchObject({
    code,
    status,
    ...(message !== undefined ? { message } : {}),
  })
}

/** Tenant with a scoped (no `automation`) token and one with a read-only token. */
export const restrictedClients = async () => {
  const wrongScope = await createTenant({ label: "ai-scope", scopes: ["contacts"] })
  const readOnly = await createTenant({ label: "ai-ro", permission: "read_only" })
  return { wrongScope: apiClient(wrongScope.token), readOnly: apiClient(readOnly.token), readOnlyTenant: readOnly }
}

export const EMBEDDING_DIMENSIONS = 1536

/** All components equal: cosine similarity 1 with itself. */
export const flatVector = (): number[] => Array.from({ length: EMBEDDING_DIMENSIONS }, () => 0.02)

/** Alternating sign: cosine similarity 0 with `flatVector()`. */
export const alternatingVector = (): number[] =>
  Array.from({ length: EMBEDDING_DIMENSIONS }, (_, index) => (index % 2 === 0 ? 0.02 : -0.02))

/** OpenAI embeddings answer for every request whose body contains `marker`. */
export const scriptEmbeddings = (marker: string, vector: number[]) =>
  fake.script({
    match: { host: "api.openai.com", method: "POST", path: "/v1/embeddings", bodyIncludes: marker },
    respond: {
      status: 200,
      json: {
        object: "list",
        data: [{ object: "embedding", index: 0, embedding: vector }],
        model: "text-embedding-ada-002",
        usage: { prompt_tokens: 8, total_tokens: 8 },
      },
    },
  })

export const embeddingRequests = async (marker: string) =>
  (await fake.requests({ kind: "http", host: "api.openai.com", pathPrefix: "/v1/embeddings" })).filter(
    (record) => record.bodyText.includes(marker),
  )

/**
 * Connects OpenAI through the public API (`PUT /v1/integrations/ai/openai`),
 * the embedding provider that aiFiles needs. The key check calls
 * `GET https://api.openai.com/v1/models`, scripted here. Setup, not a parity
 * assertion of this unit.
 */
export const connectOpenAI = async (api: Api): Promise<string> => {
  const apiKey = `sk-parity-${uniqueId()}`
  await fake.script({
    match: { host: "api.openai.com", method: "GET", path: "/v1/models" },
    respond: { status: 200, json: { object: "list", data: [{ id: "gpt-4o-mini", object: "model" }] } },
  })
  const response = await api.put("/v1/integrations/ai/openai", {
    apiKey,
    model: "gpt-4o-mini",
    temperature: 0.5,
    maxOutputTokens: 1000,
  })
  expect(response.status, response.text).toBe(200)
  return apiKey
}

export type AiTenant = Tenant & { api: Api }

export const aiTenant = async (label: string): Promise<AiTenant> => {
  const tenant = await createTenant({ label })
  return { ...tenant, api: apiClient(tenant.token) }
}

/** multipart/form-data body with text fields and one file part. */
export const multipart = (
  fields: Record<string, string>,
  file?: { field: string; fileName: string; contentType: string; content: string | Uint8Array },
) => {
  const boundary = `----parity${uniqueId()}`
  const parts: Buffer[] = []
  for (const [name, value] of Object.entries(fields)) {
    parts.push(
      Buffer.from(`--${boundary}\r\nContent-Disposition: form-data; name="${name}"\r\n\r\n${value}\r\n`),
    )
  }
  if (file) {
    parts.push(
      Buffer.from(
        `--${boundary}\r\nContent-Disposition: form-data; name="${file.field}"; filename="${file.fileName}"\r\nContent-Type: ${file.contentType}\r\n\r\n`,
      ),
      Buffer.from(file.content),
      Buffer.from("\r\n"),
    )
  }
  parts.push(Buffer.from(`--${boundary}--\r\n`))
  return {
    body: new Uint8Array(Buffer.concat(parts)),
    contentType: `multipart/form-data; boundary=${boundary}`,
  }
}

export type AiFile = {
  id: string
  name: string
  path: string
  size: number
  mimeType: string
  url: string
  chunksCount: number
  processingStatus: string
  chunking: Record<string, unknown> | null
  chunkingConfig: Record<string, unknown>
  processingRun: Record<string, unknown> | null
  [key: string]: unknown
}

/** `POST /v1/ai-files` with a multipart text file. */
export const uploadTextFile = (
  token: string,
  input: { text: string; fileName?: string; name?: string; extraFields?: Record<string, string> },
) => {
  const form = multipart(
    { ...(input.name === undefined ? {} : { name: input.name }), ...(input.extraFields ?? {}) },
    {
      field: "file",
      fileName: input.fileName ?? "knowledge.txt",
      contentType: "text/plain",
      content: input.text,
    },
  )
  return new HttpClient().request<AiFile>("POST", "/api/v1/ai-files", {
    body: form.body,
    headers: { "content-type": form.contentType },
    ...(token ? { bearer: token } : {}),
    noCookies: true,
  })
}

/** Text whose every 200-character window holds the marker (22-char units). */
export const markerText = (marker: string, units: number): string =>
  Array.from({ length: units }, () => `${marker} data.`).join(" ")

/** Waits until the worker processed the file and every chunk is embedded. */
export const waitForEmbedded = (api: Api, id: string, chunks?: number) =>
  eventually(
    async () => {
      const response = await api.get<AiFile>(`/v1/ai-files/${id}`)
      expect(response.status).toBe(200)
      expect(response.json.processingRun?.status).toBe("success")
      expect(response.json.processingStatus).toBe("success")
      if (chunks !== undefined) {
        expect(response.json.chunksCount).toBe(chunks)
      }
      return response.json
    },
    { timeoutMs: 60_000, intervalMs: 300, description: `AI file ${id} embedded` },
  )
