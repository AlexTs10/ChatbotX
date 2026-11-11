// Helpers of the P1.7 customFields / decisionConnections / decisionProfiles
// tests. Not a test file. Upstream calls of the decision connection test go
// to the fake upstream; every call a test causes is scripted.

import { expect } from "vitest"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import type { HttpResponse } from "../../src/http.ts"
import { uniqueHost, uniqueId } from "../../src/unique.ts"
import { type Api, expectError, type ErrorBody } from "./helpers.ts"

/**
 * Dotted issue paths of a 422 `invalidRequestData`. Input-schema errors carry
 * `data.issues[]`; a ZodError raised inside a handler carries the issue array
 * itself as `data`. Both forms are read.
 */
export const anyIssuePaths = (response: HttpResponse<unknown>): string[] => {
  const body = (response.json ?? {}) as ErrorBody & { data?: unknown }
  const issues = Array.isArray(body.data)
    ? (body.data as Array<{ path?: unknown[] }>)
    : ((body.data as { issues?: Array<{ path?: unknown[] }> } | undefined)?.issues ?? [])
  return issues
    .map((issue) =>
      (issue.path ?? [])
        .map((segment) =>
          segment && typeof segment === "object" ? String((segment as { key: unknown }).key) : String(segment),
        )
        .join("."),
    )
    .sort()
}

/** 422 `invalidRequestData` with an issue at `path` (either data form). */
export const expectInvalidAt = (response: HttpResponse<unknown>, path: string): void => {
  expectError(response, 422, "invalidRequestData")
  expect(anyIssuePaths(response), response.text).toContain(path)
}

// ---------------------------------------------------------------------------
// Custom fields
// ---------------------------------------------------------------------------

export type CustomField = {
  id: string
  name: string
  type: string
  description: string | null
  folderId: string | null
  showInInbox: boolean
  createdAt: string
  updatedAt: string
}

export const createField = async (
  api: Api,
  input: { name: string; type: string; description?: string | null; folderId?: string | null },
): Promise<CustomField> => {
  const response = await api.post<CustomField>("/v1/custom-fields", input)
  expect(response.status, response.text).toBe(201)
  return response.json
}

/** A `customField` (or `tag`) folder through the public folders API. */
export const createFolder = async (api: Api, name: string, folderType: "customField" | "tag" = "customField") => {
  const response = await api.post<{ id: string }>("/v1/folders", { name, folderType })
  expect([200, 201], response.text).toContain(response.status)
  return response.json.id
}

export const fieldPath = (key: string): string => `/v1/custom-fields/${encodeURIComponent(key)}`

// ---------------------------------------------------------------------------
// Decision connections
// ---------------------------------------------------------------------------

/** A valid System One Compatible answer to the conformance test of decisionConnections.test. */
export const conformanceAnswer = (model: string) => ({
  model,
  answers: {
    choice_test: { type: "choice", choice: "one", confidence: 0.9 },
    score_test: { type: "score", score: 1, confidence: 0.8 },
    noul_test: { type: "noul", probability: 0.25, confidence: 0.7 },
  },
})

export type Connection = {
  id: string
  name: string
  providerKind: "systemOneCompatible" | "typesafe" | "openrouterDecision"
  endpoint: string | null
  modelCatalog: string[]
  defaultModel: string | null
  status: "enabled" | "disabled"
  credentialConfigured: true
  lastTest: { status: "passed" | "failed" | null; testedAt: string | null }
}

export type CompatibleConnection = {
  connection: Connection
  host: string
  endpoint: string
  credential: string
  model: string
}

/** A System One Compatible connection on a unique `*.parity.test` host (create does only DoH, no HTTP). */
export const createCompatibleConnection = async (
  api: Api,
  overrides: Record<string, unknown> = {},
): Promise<CompatibleConnection> => {
  const host = uniqueHost("router")
  const endpoint = `https://${host}/v1/decide`
  const credential = `sk-parity-${uniqueId()}`
  const model = `parity-model-${uniqueId()}`
  const response = await api.post<Connection>("/v1/decision-connections", {
    name: `Parity router ${uniqueId()}`,
    providerKind: "systemOneCompatible",
    endpoint,
    modelCatalog: [model],
    defaultModel: model,
    credential,
    ...overrides,
  })
  expect(response.status, response.text).toBe(201)
  return { connection: response.json, host, endpoint, credential, model }
}

/** Scripts the provider answer for POST <path> on `host`. */
export const scriptProvider = (
  host: string,
  path: string,
  respond: Parameters<typeof fake.script>[0]["respond"],
  options: { bodyIncludes?: string; times?: number } = {},
) =>
  fake.script({
    match: { host, method: "POST", path, ...(options.bodyIncludes ? { bodyIncludes: options.bodyIncludes } : {}) },
    respond,
    ...(options.times ? { times: options.times } : {}),
  })

/** HTTP records on `host` whose body contains `marker` (when given). */
export const providerCalls = async (host: string, marker?: string): Promise<UpstreamRecord[]> =>
  (await fake.requests({ kind: "http", host })).filter((record) => !marker || record.bodyText.includes(marker))

// ---------------------------------------------------------------------------
// Decision profiles
// ---------------------------------------------------------------------------

export type Profile = {
  id: string
  connectionId: string
  name: string
  description: string | null
  model: string
  status: "enabled" | "disabled"
  decision: Record<string, unknown>
  thresholdConfig: Record<string, unknown> | null
  globalRouterEnabled: boolean
  globalRouterConfig: Record<string, unknown> | null
}

export const choiceDecision = {
  type: "choice",
  instructions: "Pick the intent of the message.",
  options: [
    { value: "buy", description: "The customer wants to buy." },
    { value: "support", description: "The customer needs help." },
  ],
}

export const profileInput = (connectionId: string, model: string, overrides: Record<string, unknown> = {}) => ({
  name: `Parity profile ${uniqueId()}`,
  connectionId,
  model,
  decision: choiceDecision,
  ...overrides,
})

export const createProfile = async (
  api: Api,
  connectionId: string,
  model: string,
  overrides: Record<string, unknown> = {},
): Promise<Profile> => {
  const response = await api.post<Profile>("/v1/decision-profiles", profileInput(connectionId, model, overrides))
  expect(response.status, response.text).toBe(201)
  return response.json
}

/** Global Router config defaults the API fills in. */
export const routerConfig = (fallback: { type: string; id: string }, extra: Record<string, unknown> = {}) => ({
  fallback,
  ...extra,
})

export const filledRouterConfig = (fallback: { type: string; id: string }, extra: Record<string, unknown> = {}) => ({
  fallback,
  metadataMappings: [],
  noul: { falseMaximum: 0.5, trueMinimum: 0.5 },
  routes: [],
  version: 1,
  ...extra,
})
