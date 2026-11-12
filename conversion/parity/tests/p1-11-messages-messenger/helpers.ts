// Shared helpers of unit P1.11 (magicLinks, me, mediaLibrary, messages,
// messengerChannels, messengerPersonas, messengerTemplates). Not a test file.
// Copied in part from tests/p1-10-inboxes-keywords-leaderboards/helpers.ts
// (units do not share files). Setup goes through public routes where one
// exists and the P1.11 fixture extension where none does; every assertion
// stays black-box.

import { expect } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import { loadFixtureExt } from "../../src/fixtures/ext.ts"
import type { P111Fixtures } from "../../src/fixtures/ext/p1-11.ts"
import { createTenant, fixtures, signedInClient, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, HttpClient, type HttpResponse } from "../../src/http.ts"
import { connectRealtime, type RealtimeSocket, workspaceSocketUrl } from "../../src/realtime.ts"
import { rpc } from "../../src/rpc.ts"
import { uniqueHost, uniqueId } from "../../src/unique.ts"

export type Api = ReturnType<typeof apiClient>

export const ext = (): Promise<P111Fixtures> => loadFixtureExt<P111Fixtures>("p1-11")

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
/** A unique all-digit id (page ids, phone number ids, comment ids are numeric). */
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

/** 422 `invalidRequestData` without checking the issue path (union / intersection inputs). */
export const expectInvalidAny = (response: HttpResponse<unknown>): void => {
  expectError(response, 422, "invalidRequestData", { defined: true })
}

/** 404 `notFound` with a fixed message. */
export const expectNotFound = (response: HttpResponse<unknown>, message: string, defined = true): void => {
  expectError(response, 404, "notFound", { defined, message })
}

/** 400 `systemError` (a bare `ChatbotXException`) with its fixed message. */
export const expectSystemError = (response: HttpResponse<unknown>, message: string): void => {
  expectError(response, 400, "systemError", { message })
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

/** Waits until at least `count` matching records exist, then returns them. */
export const waitForCalls = (
  input: Parameters<typeof upstreamCalls>[0],
  count = 1,
  timeoutMs = 30_000,
): Promise<UpstreamRecord[]> =>
  wait(async () => {
    const records = await upstreamCalls(input)
    expect(records.length, `${count} call(s) to ${input.host} ${input.path ?? ""}`).toBeGreaterThanOrEqual(count)
    return records
  }, timeoutMs)

export const parseBody = <T = Record<string, unknown>>(record: UpstreamRecord): T => JSON.parse(record.bodyText) as T

export const graphError = (message: string, code = 100) => ({
  error: { message, type: "OAuthException", code, fbtrace_id: "parity-trace" },
})

// ---------------------------------------------------------------------------
// Flows
// ---------------------------------------------------------------------------

/** A published flow whose steps send the given texts. */
export const createPublishedFlow = async (api: Api, name: string, ...texts: string[]): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/flows", {
    folderId: null,
    name,
    publish: true,
    spec: { formatVersion: 1, name, steps: (texts.length > 0 ? texts : ["Parity flow"]).map((text) => ({ type: "send", text })) },
  })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

// ---------------------------------------------------------------------------
// Channels and conversations
// ---------------------------------------------------------------------------

export type MessengerPage = {
  integrationId: string
  inboxId: string
  pageId: string
  name: string
  accessToken: string
}

/** A connected Messenger page through the fixture extension. */
export const connectMessenger = async (
  tenant: Tenant,
  extra: Omit<Parameters<P111Fixtures["createMessengerPage"]>[0], "workspaceId" | "pageId" | "name"> = {},
): Promise<MessengerPage> => {
  const pageId = uniqueDigits()
  const name = uniqueName("FB Page")
  const created = await (await ext()).createMessengerPage({ workspaceId: tenant.workspaceId, pageId, name, ...extra })
  // Publishing a flow in a workspace with Messenger pages mirrors it to each
  // page as a Meta welcome message flow (worker job a few seconds later).
  await fake.script({
    match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${pageId}/welcome_message_flows`) },
    respond: { status: 200, json: { flow_id: `wmf-${pageId}` } },
  })
  return { ...created, pageId, name }
}

export type Conversation = {
  contactId: string
  contactInboxId: string
  conversationId: string
  sourceId: string
  firstName: string
}

/** A contact on one inbox with one conversation, through the fixture extension. */
export const seedConversation = async (
  tenant: Tenant,
  inbox: { inboxId: string; channel: string; sourceId?: string; lastIncomingMessageAt?: string | null },
): Promise<Conversation> => {
  const firstName = uniqueName("Ada")
  const created = await (await ext()).createContact({
    workspaceId: tenant.workspaceId,
    firstName,
    inboxes: [inbox],
  })
  return {
    contactId: created.contactId,
    contactInboxId: created.contactInboxIds[0] as string,
    conversationId: created.conversationId,
    sourceId: created.sourceIds[0] as string,
    firstName,
  }
}

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

/** A webchat inbox plus one conversation on it (no third-party call on send). */
export const webchatConversation = async (tenant: Tenant, api: Api): Promise<Conversation & Webchat> => {
  const webchat = await createWebchat(api)
  const conversation = await seedConversation(tenant, { inboxId: webchat.inboxId, channel: "webchat" })
  return { ...webchat, ...conversation }
}

export type ApiChannel = { inboxId: string; callbackHost: string }

/**
 * An API channel whose outbound callback goes to a new `*.parity.test` host
 * on the fake upstream, which answers 200 `{"messageId": ...}` to every POST.
 */
export const createApiChannel = async (api: Api): Promise<ApiChannel> => {
  const callbackHost = uniqueHost("p111-cb")
  await fake.script({
    match: { host: callbackHost, method: "POST", path: "/hook" },
    respond: { status: 200, json: { messageId: `ext-${uniqueId()}` } },
  })
  const created = await api.post<{ connection: { inboxId: string } }>("/v1/connections", {
    provider: "api",
    config: { name: uniqueName("p111").slice(0, 40), callbackUrl: `https://${callbackHost}/hook` },
  })
  expect(created.status, created.text).toBe(201)
  return { inboxId: created.json.connection.inboxId, callbackHost }
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

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

export const MESSAGE_KEYS = [
  "attachments",
  "attributes",
  "contactInboxId",
  "contentAttributes",
  "contentType",
  "conversationId",
  "createdAt",
  "deletedAt",
  "id",
  "messageType",
  "parentId",
  "sendError",
  "senderId",
  "senderType",
  "sourceId",
  "text",
  "type",
  "updatedAt",
  "workspaceId",
]

export type Message = {
  id: string
  createdAt: string
  conversationId: string
  contactInboxId: string
  workspaceId: string
  text: string | null
  messageType: string
  senderType: string
  senderId: string | null
  sourceId: string | null
  type: string
  parentId: string | null
  attributes: { liked: boolean; hidden: boolean } | null
  contentAttributes: Record<string, unknown> | null
  deletedAt: string | null
  sendError: string | null
  attachments: Array<Record<string, unknown>>
  [key: string]: unknown
}

export type MessagePage = { data: Message[]; nextCursor: string | null; prevCursor: string | null }

export const messagesPath = (conversationId: string, suffix = ""): string =>
  `/v1/conversations/${conversationId}/messages${suffix}`

export const listMessages = async (
  api: Api,
  conversationId: string,
  query: Record<string, string | number | undefined> = {},
): Promise<MessagePage> => {
  const response = await api.get<MessagePage>(messagesPath(conversationId), { query })
  expect(response.status, response.text).toBe(200)
  return response.json
}

export const getMessage = (api: Api, conversationId: string, message: { id: string; createdAt: string }) =>
  api.get<Message>(messagesPath(conversationId, `/${message.id}`), { query: { createdAt: message.createdAt } })

// ---------------------------------------------------------------------------
// Mobile (me) tokens
// ---------------------------------------------------------------------------

export type MobileTokens = {
  tokenType: "Bearer"
  accessToken: string
  expiresIn: number
  expiresAt: number
  refreshToken: string
  refreshTokenExpiresAt: number
  deviceId: string
  mustChangePassword: boolean
}

/** `POST /api/auth/mobile/token` (password grant), from a new client address. */
export const mobileToken = async (
  input: { email: string; password: string; deviceId?: string },
  http: HttpClient = new HttpClient(),
): Promise<HttpResponse<MobileTokens>> =>
  http.post<MobileTokens>("/api/auth/mobile/token", { grantType: "password", ...input })

export const mobileTokenOrThrow = async (input: { email: string; password: string; deviceId?: string }) => {
  const response = await mobileToken(input)
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

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

/** A 1x1 PNG. */
export const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
  "base64",
)

/** GET of a public storage URL (`http://app/storage/...`) without credentials. */
export const fetchPublic = async (url: string): Promise<{ status: number; bytes: Buffer; contentType: string | null }> => {
  const response = await fetch(url)
  const bytes = Buffer.from(await response.arrayBuffer())
  return { status: response.status, bytes, contentType: response.headers.get("content-type") }
}

export type UploadedFile = { id: string; path: string; url: string; name: string; mimeType: string; size: number }

/** createUploadUrl -> PUT the bytes -> createFile. Returns the registered file. */
export const uploadMediaFile = async (
  api: Api,
  input: { name?: string; mimeType?: string; bytes?: Buffer; folderId?: string | null } = {},
): Promise<UploadedFile> => {
  const name = input.name ?? `${uniqueName("photo")}.png`
  const mimeType = input.mimeType ?? "image/png"
  const bytes = input.bytes ?? PNG_1X1
  const presigned = await api.post<{ path: string; uploadUrl: string; publicUrl: string }>(
    "/v1/media-library/files/upload-url",
    { fileName: name, mimeType },
  )
  expect(presigned.status, presigned.text).toBe(201)
  const put = await fetch(presigned.json.uploadUrl, { method: "PUT", body: bytes })
  expect(put.status).toBe(200)
  const created = await api.post<UploadedFile>("/v1/media-library/files", {
    name,
    path: presigned.json.path,
    mimeType,
    size: bytes.length,
    ...(input.folderId !== undefined ? { folderId: input.folderId } : {}),
  })
  expect(created.status, created.text).toBe(201)
  return created.json
}
