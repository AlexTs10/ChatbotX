// Shared helpers of the P1.9 inbound-webhook tests (files 2*.test.ts). Not a
// test file. Copied in part from tests/p1-05-channels-connections/helpers.ts
// and tests/p1-06-contacts/helpers.ts (units do not share files). Setup goes
// through public HTTP routes (connections, webchats, contacts, tags, custom
// fields, flows); every assertion stays black-box.

import { expect } from "vitest"
import { config } from "../../src/config.ts"
import { eventually } from "../../src/eventually.ts"
import { fake } from "../../src/fake.ts"
import { createTenant, fixtures, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, HttpClient, type HttpResponse } from "../../src/http.ts"
import { uniqueHost, uniqueId } from "../../src/unique.ts"

export type Api = ReturnType<typeof apiClient>

export const tenantApi = async (
  label: string,
  options: Parameters<typeof createTenant>[0] = {},
): Promise<{ tenant: Tenant; api: Api }> => {
  const tenant = await createTenant({ label, ...options })
  return { tenant, api: apiClient(tenant.token) }
}

/** An extra token on the tenant's workspace (read-only or scoped). */
export const extraToken = async (
  tenant: Tenant,
  options: { permission?: "full" | "read_only"; scopes?: string[] | null },
): Promise<string> => {
  const driver = await fixtures()
  const { token } = await driver.createWorkspaceToken({
    workspaceId: tenant.workspaceId,
    name: `parity-${options.permission ?? "full"}`,
    permission: options.permission ?? "full",
    scopes: options.scopes === undefined ? null : options.scopes,
  })
  return token
}

export const uniqueName = (prefix: string): string => `${prefix}-${uniqueId()}`

/** Status, oRPC error `code` and (optionally) `defined` / `message` of an error response. */
export const expectError = (
  response: HttpResponse,
  status: number,
  code: string,
  options: { defined?: boolean; message?: string } = {},
): void => {
  const body = (response.json ?? {}) as Record<string, unknown>
  expect({ status: response.status, code: body.code }, response.text).toEqual({ status, code })
  expect(body.status).toBe(status)
  if (options.defined !== undefined) {
    expect(body.defined).toBe(options.defined)
  }
  if (options.message !== undefined) {
    expect(body.message).toBe(options.message)
  }
}

/** Issue paths of a 422 `invalidRequestData` error, each as a dotted string. */
export const issuePaths = (response: HttpResponse): string[] => {
  const data = (response.json as { data?: { issues?: Array<{ path?: unknown[] }> } }).data
  return (data?.issues ?? []).map((issue) =>
    (issue.path ?? [])
      .map((segment) => (segment && typeof segment === "object" ? (segment as { key: unknown }).key : segment))
      .join("."),
  )
}

/** 422 `invalidRequestData` with an issue at `path` (dotted). */
export const expectInvalid = (response: HttpResponse, path: string): void => {
  expectError(response, 422, "invalidRequestData", { defined: true })
  expect(issuePaths(response), response.text).toContain(path)
}

/** 422 `validation` (business validation) with its i18n key as the message. */
export const expectValidation = (response: HttpResponse, message: string): void => {
  expectError(response, 422, "validation", { defined: true, message })
}

/** Asserts an ISO-8601 UTC time string that lies within the test window. */
export const expectIso = (value: unknown, start: number): void => {
  expect(value).toEqual(expect.stringMatching(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z$/))
  const time = Date.parse(value as string)
  expect(time).toBeGreaterThanOrEqual(start - 5_000)
  expect(time).toBeLessThanOrEqual(Date.now() + 5_000)
}

export const SNOWFLAKE = /^\d{15,20}$/
export const ENDPOINT_URL = new RegExp(`^${config.baseUrl.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}/api/hooks/iwh_[A-Za-z0-9_-]{32,64}$`)

// ---------------------------------------------------------------------------
// Inbound webhook resources
// ---------------------------------------------------------------------------

export type InboxResolution =
  | { mode: "inbox"; inboxId: string }
  | { mode: "payload"; channel: string; identifierKind: string; jsonPath: string }

export type Webhook = {
  id: string
  name: string
  active: boolean
  createdAt: string
  updatedAt: string
  requestCount: number
  endpointUrl?: string
  inboxResolution: InboxResolution | null
  identifiers: Array<{ kind: string; jsonPath: string }>
  fieldMappings: Array<{ jsonPath: string; target: string }>
  actions: Array<Record<string, unknown>>
  sampleJson: unknown
}

export type WebhookSummary = Pick<Webhook, "id" | "name" | "active" | "createdAt" | "updatedAt" | "requestCount">

export type WebhookRequest = {
  id: string
  status: string
  skipReason: string | null
  errorCode: string | null
  errorMessage: string | null
  inboxId: string | null
  contactId: string | null
  attempts: number
  receivedAt: string
  processedAt: string | null
  fieldErrors?: Array<{ target: string; code: string }> | null
  actionResults?: Array<Record<string, unknown>> | null
  payload?: unknown
}

export const SUMMARY_KEYS = ["active", "createdAt", "id", "name", "requestCount", "updatedAt"]
export const RESOURCE_KEYS = [
  ...SUMMARY_KEYS,
  "actions",
  "endpointUrl",
  "fieldMappings",
  "identifiers",
  "inboxResolution",
  "sampleJson",
].sort()
export const REQUEST_SUMMARY_KEYS = [
  "attempts",
  "contactId",
  "errorCode",
  "errorMessage",
  "id",
  "inboxId",
  "processedAt",
  "receivedAt",
  "skipReason",
  "status",
]
export const REQUEST_DETAIL_KEYS = [...REQUEST_SUMMARY_KEYS, "actionResults", "fieldErrors"].sort()

export const createWebhook = async (api: Api, name = uniqueName("iwh")): Promise<Webhook> => {
  const response = await api.post<Webhook>("/v1/inbound-webhooks", { name })
  expect(response.status, response.text).toBe(201)
  return response.json
}

export type WebhookConfig = {
  inboxResolution: InboxResolution | null
  identifiers: Array<{ kind: string; jsonPath: string }>
  fieldMappings: Array<{ jsonPath: string; target: string }>
  actions: Array<Record<string, unknown>>
  sampleJson?: unknown
}

/** `sampleJson` must be present in the body (null clears it); omitted here, it is sent as null. */
export const putConfig = (api: Api, id: string, config: WebhookConfig) =>
  api.put<Webhook>(`/v1/inbound-webhooks/${id}`, { ...config, sampleJson: config.sampleJson ?? null })

export const configure = async (api: Api, id: string, config: WebhookConfig): Promise<Webhook> => {
  const response = await putConfig(api, id, config)
  expect(response.status, response.text).toBe(200)
  return response.json
}

export const setActive = async (api: Api, id: string, active: boolean): Promise<Webhook> => {
  const response = await api.patch<Webhook>(`/v1/inbound-webhooks/${id}/settings`, { active })
  expect(response.status, response.text).toBe(200)
  return response.json
}

/** A configured and active webhook; returns its resource (with endpointUrl). */
export const activeWebhook = async (api: Api, config: WebhookConfig, name?: string): Promise<Webhook> => {
  const created = await createWebhook(api, name)
  await configure(api, created.id, config)
  return await setActive(api, created.id, true)
}

/** Path part of an endpoint URL (`/api/hooks/iwh_...`), sent through `config.baseUrl`. */
export const hookPath = (endpointUrl: string): string => new URL(endpointUrl).pathname

/** A client of the public hook route (no token; its own client address). */
export const hookClient = (): HttpClient => new HttpClient()

export const deliver = (
  endpointUrl: string,
  payload: unknown,
  options: { client?: HttpClient; headers?: Record<string, string> } = {},
) => (options.client ?? hookClient()).post<Record<string, unknown>>(hookPath(endpointUrl), payload, {
  headers: options.headers,
})

/** Delivers a payload and returns the request id of the 202 answer. */
export const deliverAccepted = async (endpointUrl: string, payload: unknown): Promise<string> => {
  const response = await deliver(endpointUrl, payload)
  expect(response.status, response.text).toBe(202)
  expect(response.json).toEqual({ accepted: true, requestId: expect.stringMatching(SNOWFLAKE) })
  return response.json.requestId as string
}

const TERMINAL = new Set(["processed", "completedWithErrors", "skipped", "failed"])

/** Polls `getRequest` until the request reaches a terminal status. */
export const waitForRequest = async (
  api: Api,
  webhookId: string,
  requestId: string,
  options: { includePayload?: boolean; timeoutMs?: number } = {},
): Promise<WebhookRequest> =>
  await eventually(
    async () => {
      const response = await api.get<WebhookRequest>(
        `/v1/inbound-webhooks/${webhookId}/requests/${requestId}${options.includePayload ? "?includePayload=true" : ""}`,
      )
      expect(response.status, response.text).toBe(200)
      expect(TERMINAL.has(response.json.status), `status ${response.json.status}`).toBe(true)
      return response.json
    },
    { timeoutMs: options.timeoutMs ?? 45_000, intervalMs: 300, description: `inbound webhook request ${requestId}` },
  )

// ---------------------------------------------------------------------------
// Setup through public routes of other namespaces (not parity assertions here)
// ---------------------------------------------------------------------------

export type ApiChannel = { inboxId: string; connectionId: string; callbackHost: string | null }

/**
 * An API channel (`connections.create {provider:"api"}`). With
 * `callback: true` its callback URL points at a unique `*.parity.test` host
 * of the fake upstream, scripted to answer 200.
 */
export const createApiChannel = async (api: Api, options: { callback?: boolean } = {}): Promise<ApiChannel> => {
  const callbackHost = options.callback ? uniqueHost("iwh-cb") : null
  if (callbackHost) {
    await fake.script({
      match: { host: callbackHost, method: "POST", path: "/hook" },
      respond: { status: 200, json: { messageId: `ext-${uniqueId()}` } },
    })
  }
  const response = await api.post<{ connection: { id: string; inboxId: string } }>("/v1/connections", {
    provider: "api",
    config: {
      name: uniqueName("iwh").slice(0, 40),
      ...(callbackHost ? { callbackUrl: `https://${callbackHost}/hook` } : {}),
    },
  })
  expect(response.status, response.text).toBe(201)
  return { inboxId: response.json.connection.inboxId, connectionId: response.json.connection.id, callbackHost }
}

export const createWebchat = async (api: Api): Promise<{ id: string; inboxId: string }> => {
  const response = await api.post<{ id: string; inboxId: string }>("/v1/webchats", {
    name: uniqueName("iwh-webchat"),
    brandColor: "#007bff",
    conversationStarters: [],
    persistentMenus: [],
    authorizedDomains: ["example.com"],
  })
  expect(response.status, response.text).toBe(201)
  return { id: response.json.id, inboxId: response.json.inboxId }
}

export const createWebchatContact = async (
  api: Api,
  inboxId: string,
  input: { firstName: string; lastName: string; email?: string },
): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/contacts", {
    channel: "webchat",
    inboxId,
    firstName: input.firstName,
    lastName: input.lastName,
    email: input.email ?? "",
    gender: "unknown",
  })
  expect(response.status, response.text).toBe(200)
  return response.json.id
}

export const createTag = async (api: Api, name = uniqueName("tag")): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/tags", { name })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

export const createCustomField = async (
  api: Api,
  name: string,
  type: "shortText" | "number" | "date" | "boolean" | "email",
): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/custom-fields", { name, type })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

/** A flow with one text step; published unless `publish: false` (draft only). */
export const createFlow = async (
  api: Api,
  input: { name?: string; text: string; publish?: boolean },
): Promise<string> => {
  const name = input.name ?? uniqueName("iwh-flow")
  const response = await api.post<{ id: string }>("/v1/flows", {
    name,
    folderId: null,
    spec: { formatVersion: 1, name, steps: [{ type: "send", text: input.text }] },
    publish: input.publish ?? true,
  })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

export type Contact = {
  id: string
  firstName: string | null
  lastName: string | null
  fullName: string | null
  email: string | null
  phoneNumber: string | null
  gender: string | null
  contactInboxes?: Array<{ id: string; channel: string; source: string; sourceId: string; inboxId: string }>
  conversation?: { id: string } | null
  [key: string]: unknown
}

export const contactPath = (contactId: string, suffix = ""): string =>
  `/v1/contacts/${encodeURIComponent(`id:${contactId}`)}${suffix}`

export const getContact = async (api: Api, contactId: string): Promise<Contact> => {
  const response = await api.get<Contact>(contactPath(contactId))
  expect(response.status, response.text).toBe(200)
  return response.json
}

export const contactTagNames = async (api: Api, contactId: string): Promise<string[]> => {
  const response = await api.get<{ data: Array<{ name: string }> }>(contactPath(contactId, "/tags"))
  expect(response.status, response.text).toBe(200)
  return response.json.data.map((tag) => tag.name).sort()
}

export const contactFieldValues = async (api: Api, contactId: string): Promise<Record<string, string>> => {
  const response = await api.get<{ data: Array<{ name: string; value: string }> }>(contactPath(contactId, "/custom-fields"))
  expect(response.status, response.text).toBe(200)
  return Object.fromEntries(response.json.data.map((row) => [row.name, row.value]))
}
