// Shared helpers of unit P1.14 (productCategories, productVendors, products,
// qrCodes, questionnaires, realtime, reflinks). Not a test file. Copied in
// part from tests/p1-13-pipelines-places-polls/helpers.ts and
// tests/p1-11-messages-messenger/helpers.ts (units do not share files).
// Setup goes through public routes where one exists and the P1.12 fixture
// extension (shared by P1.12–P1.14) where none does; every assertion stays
// black-box.

import { expect } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import { loadFixtureExt } from "../../src/fixtures/ext.ts"
import type { P112Fixtures } from "../../src/fixtures/ext/p1-12.ts"
import { createTenant, fixtures, signedInClient, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, HttpClient, type HttpResponse } from "../../src/http.ts"
import { connectRealtime, type RealtimeSocket, workspaceSocketUrl } from "../../src/realtime.ts"
import { rpc } from "../../src/rpc.ts"
import { uniqueId } from "../../src/unique.ts"

export type Api = ReturnType<typeof apiClient>

export const ext = (): Promise<P112Fixtures> => loadFixtureExt<P112Fixtures>("p1-12")

export const GRAPH = "graph.facebook.com"

export const newTenant = async (
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
): Promise<Api> => {
  const driver = await fixtures()
  const { token } = await driver.createWorkspaceToken({
    workspaceId: tenant.workspaceId,
    name: `parity-${options.permission ?? "full"}`,
    permission: options.permission ?? "full",
    scopes: options.scopes === undefined ? null : options.scopes,
  })
  return apiClient(token)
}

export const uniqueName = (prefix: string): string => `${prefix}-${uniqueId()}`

let numericCounter = 0
/** A unique all-digit id (Meta phone number ids are numeric). */
export const uniqueDigits = (): string => {
  numericCounter = (numericCounter + 1) % 1000
  return `${Date.now()}${String(numericCounter).padStart(3, "0")}${Math.floor(Math.random() * 1000)
    .toString()
    .padStart(3, "0")}`
}

/** A numeric id that no row has. */
export const UNKNOWN_ID = "999999999999999999"

export const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/
export const SNOWFLAKE = /^\d{15,20}$/

/** Async effects (worker jobs, schedulers). */
export const wait = <T>(fn: () => Promise<T> | T, timeoutMs = 45_000): Promise<T> =>
  eventually(fn, { timeoutMs, intervalMs: 500 })

export const keysOf = (value: unknown): string[] => Object.keys((value ?? {}) as object).sort()

type ErrorBody = {
  defined?: boolean
  code?: string
  status?: number
  message?: string
  data?: unknown
}

/** Status + oRPC error code (+ optional `defined` / message). Returns the body. */
export const expectError = (
  response: HttpResponse<unknown>,
  status: number,
  code: string,
  options: { defined?: boolean; message?: string } = {},
): ErrorBody => {
  const body = (response.json ?? {}) as ErrorBody
  expect({ httpStatus: response.status, code: body.code, status: body.status }, response.text).toEqual({
    httpStatus: status,
    code,
    status,
  })
  if (options.defined !== undefined) {
    expect(body.defined, response.text).toBe(options.defined)
  }
  if (options.message !== undefined) {
    expect(body.message, response.text).toBe(options.message)
  }
  return body
}

/** Issue paths of a 422 `invalidRequestData` from oRPC input validation (`data.issues`). */
export const issuePaths = (response: HttpResponse<unknown>): string[] => {
  const data = (response.json as { data?: { issues?: Array<{ path?: unknown[] }> } } | undefined)?.data
  return (data?.issues ?? []).map((issue) =>
    (issue.path ?? [])
      .map((segment) => (segment && typeof segment === "object" ? (segment as { key: unknown }).key : segment))
      .join("."),
  )
}

/** 422 `invalidRequestData` (input validation) with an issue at `path`. */
export const expectInvalid = (response: HttpResponse<unknown>, path: string): void => {
  expectError(response, 422, "invalidRequestData", { defined: true })
  expect(issuePaths(response), response.text).toContain(path)
}

/** 404 `notFound` with a fixed message. */
export const expectNotFound = (response: HttpResponse<unknown>, message: string, defined = true): void => {
  expectError(response, 404, "notFound", { defined, message })
}

/** 422 business `validation` (`validationException`) with its message. */
export const expectValidation = (response: HttpResponse<unknown>, message: string): void => {
  expectError(response, 422, "validation", { defined: true, message })
}

/** Graph API path regex that accepts any `vNN.N` version prefix. */
export const graphPath = (rest: string): string => `^/v[0-9.]+/${rest}$`

export const reEscape = (value: string): string => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")

/** HTTP records of the fake upstream for one host, after `since`, filtered by path regex / method. */
export const upstreamCalls = async (input: {
  host: string
  since: number
  path?: RegExp
  method?: string
  bearer?: string
}): Promise<UpstreamRecord[]> => {
  const records = await fake.requests({ kind: "http", host: input.host, since: input.since })
  return records.filter(
    (record) =>
      (!input.path || input.path.test(record.path)) &&
      (!input.method || record.method === input.method.toUpperCase()) &&
      (!input.bearer || record.headers.authorization === `Bearer ${input.bearer}`),
  )
}

export const parseBody = <T = Record<string, unknown>>(record: UpstreamRecord): T => JSON.parse(record.bodyText) as T

export const graphError = (message: string, code = 100) => ({
  error: { message, type: "OAuthException", code, fbtrace_id: "parity-trace" },
})

// ---------------------------------------------------------------------------
// Contacts and products (public routes)
// ---------------------------------------------------------------------------

export type Webchat = { id: string; inboxId: string }

export const createWebchat = async (api: Api, name = uniqueName("Parity webchat")): Promise<Webchat> => {
  const response = await api.post<Webchat>("/v1/webchats", {
    name,
    brandColor: "#007bff",
    conversationStarters: [],
    persistentMenus: [],
    authorizedDomains: ["example.com"],
  })
  expect(response.status, response.text).toBe(201)
  return { id: response.json.id, inboxId: response.json.inboxId }
}

/** A webchat contact made with the public `contacts.create` (contact + inbox + conversation). */
export const createWebchatContact = async (
  api: Api,
  inboxId: string,
  input: { firstName: string; lastName?: string; email?: string; gender?: "male" | "female" | "unknown" },
): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/contacts", {
    channel: "webchat",
    inboxId,
    firstName: input.firstName,
    lastName: input.lastName ?? "Parity",
    email: input.email ?? "",
    gender: input.gender ?? "unknown",
  })
  expect(response.status, response.text).toBe(200)
  return response.json.id
}

/** A webchat inbox plus one contact on it; returns the contact id. */
export const newContact = async (
  api: Api,
  firstName = uniqueName("Buyer"),
  gender: "male" | "female" | "unknown" = "unknown",
): Promise<string> => {
  const { inboxId } = await createWebchat(api, uniqueName("Chat"))
  return await createWebchatContact(api, inboxId, { firstName, gender })
}

/** The tenant owner's workspace member id (`workspaceMembers.list`). */
export const ownerMemberId = async (api: Api): Promise<string> => {
  const response = await api.get<{ data: Array<{ id: string }> }>("/v1/members")
  expect(response.status, response.text).toBe(200)
  expect(response.json.data).toHaveLength(1)
  return (response.json.data[0] as { id: string }).id
}

/** A member realtime socket of the tenant's workspace (session cookie -> connect token). */
export const memberSocket = async (tenant: Tenant): Promise<RealtimeSocket> => {
  const http = await signedInClient(tenant)
  const minted = await rpc<{ token: string }>(
    http,
    "realtimeAPI/mintWorkspaceConnectTokenAuthenticatedAPI",
    { workspaceId: tenant.workspaceId },
    { method: "GET" },
  )
  expect(minted.status).toBe(200)
  const socket = connectRealtime(workspaceSocketUrl(tenant.workspaceId, minted.json.token))
  await socket.opened
  return socket
}

/** A 1x1 PNG. */
export const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
  "base64",
)

export type UploadedFile = { id: string; path: string; url: string; name: string; mimeType: string; size: number }

/** mediaLibrary createUploadUrl -> PUT the bytes -> createFile. Returns the registered file. */
export const uploadMediaFile = async (api: Api, name = `${uniqueName("doc")}.png`): Promise<UploadedFile> => {
  const presigned = await api.post<{ path: string; uploadUrl: string }>("/v1/media-library/files/upload-url", {
    fileName: name,
    mimeType: "image/png",
  })
  expect(presigned.status, presigned.text).toBe(201)
  const put = await fetch(presigned.json.uploadUrl, { method: "PUT", body: PNG_1X1 })
  expect(put.status).toBe(200)
  const created = await api.post<UploadedFile>("/v1/media-library/files", {
    name,
    path: presigned.json.path,
    mimeType: "image/png",
    size: PNG_1X1.length,
  })
  expect(created.status, created.text).toBe(201)
  return created.json
}

/** A published flow with one text step (no Messenger page in these tests, so no Meta call). */
export const createPublishedFlow = async (api: Api, name = uniqueName("Flow")): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/flows", {
    folderId: null,
    name,
    publish: true,
    spec: { formatVersion: 1, name, steps: [{ type: "send", text: "Parity flow" }] },
  })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

/** A product made with the public `products.create`. */
export const createProduct = async (api: Api, body: Record<string, unknown> = {}): Promise<Record<string, unknown> & { id: string; name: string }> => {
  const response = await api.post<Record<string, unknown> & { id: string; name: string }>("/v1/products", {
    name: uniqueName("Product"),
    price: 10,
    ...body,
  })
  expect(response.status, response.text).toBe(200)
  return response.json
}

export type MobileTokens = { tokenType: "Bearer"; accessToken: string; refreshToken: string; deviceId: string }

/** `POST /api/auth/mobile/token` (password grant) from a new client address. */
export const mobileTokenOrThrow = async (input: { email: string; password: string }): Promise<MobileTokens> => {
  const response = await new HttpClient().post<MobileTokens>("/api/auth/mobile/token", { grantType: "password", ...input })
  expect(response.status, response.text).toBe(200)
  return response.json
}

/** A public API client that sends a mobile access JWT (+ optional `x-workspace-id`). */
export const mobileClient = (accessToken: string, workspaceId?: string): Api => {
  const http = new HttpClient()
  const withAuth = (request: Parameters<HttpClient["get"]>[1] = {}) => ({
    ...request,
    bearer: accessToken,
    noCookies: true,
    headers: { ...(workspaceId ? { "x-workspace-id": workspaceId } : {}), ...(request.headers ?? {}) },
  })
  return {
    http,
    get: (path, request) => http.get(`/api${path}`, withAuth(request)),
    post: (path, json, request) => http.post(`/api${path}`, json, withAuth(request)),
    put: (path, json, request) => http.put(`/api${path}`, json, withAuth(request)),
    patch: (path, json, request) => http.patch(`/api${path}`, json, withAuth(request)),
    delete: (path, request) => http.delete(`/api${path}`, withAuth(request)),
  } as Api
}

/** Bracket-notation query string for oRPC (`sort[0][id]=name&sort[0][desc]=true`). */
export const sortQuery = (id: string, desc: boolean): string =>
  `sort[0][id]=${encodeURIComponent(id)}&sort[0][desc]=${desc}`
