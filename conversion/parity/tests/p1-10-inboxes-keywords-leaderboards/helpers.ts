// Shared helpers of unit P1.10 (inboxes, instagramChannels, integrations,
// keywords, leaderboards). Not a test file. Copied in part from
// tests/p1-09-flows-webhooks-ig/helpers*.ts (units do not share files).
// Setup goes through public routes where one exists and the P1.10 fixture
// extension where none does; every assertion stays black-box.

import { expect } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import { loadFixtureExt } from "../../src/fixtures/ext.ts"
import type { P110Fixtures } from "../../src/fixtures/ext/p1-10.ts"
import { createTenant, fixtures, signedInClient, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, type HttpClient, type HttpResponse } from "../../src/http.ts"
import { connectRealtime, type RealtimeSocket, workspaceSocketUrl } from "../../src/realtime.ts"
import { rpc } from "../../src/rpc.ts"
import { uniqueHost, uniqueId } from "../../src/unique.ts"

export type Api = ReturnType<typeof apiClient>

export const ext = (): Promise<P110Fixtures> => loadFixtureExt<P110Fixtures>("p1-10")

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
/** A unique all-digit id (Instagram account / page ids are numeric). */
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

/** 422 business validation (`validationException`), optionally with its fixed message. */
export const expectValidation = (response: HttpResponse<unknown>, message?: string): void => {
  expectError(response, 422, "validation", message === undefined ? { defined: true } : { defined: true, message })
}

/** 404 `notFound` with a fixed message. */
export const expectNotFound = (response: HttpResponse<unknown>, message: string, defined = true): void => {
  expectError(response, 404, "notFound", { defined, message })
}

/** Graph API path regex that accepts any `vNN.N` version prefix. */
export const graphPath = (rest: string): string => `^/v[0-9.]+/${rest}$`

/** HTTP records of the fake upstream for one host, after `since`, filtered by path regex / method. */
export const upstreamCalls = async (input: {
  host: string
  since: number
  path?: RegExp
  method?: string
}): Promise<UpstreamRecord[]> => {
  const records = await fake.requests({ kind: "http", host: input.host, since: input.since })
  return records.filter(
    (record) =>
      (!input.path || input.path.test(record.path)) && (!input.method || record.method === input.method.toUpperCase()),
  )
}

export const parseBody = <T = Record<string, unknown>>(record: UpstreamRecord): T => JSON.parse(record.bodyText) as T

// ---------------------------------------------------------------------------
// Flows
// ---------------------------------------------------------------------------

/** A published flow whose steps send the given texts. */
export const createPublishedFlow = async (api: Api, name: string, ...texts: string[]): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/flows", {
    folderId: null,
    name,
    publish: true,
    spec: { formatVersion: 1, name, steps: texts.map((text) => ({ type: "send", text })) },
  })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

// ---------------------------------------------------------------------------
// Instagram accounts
// ---------------------------------------------------------------------------

export type IgAccount = {
  integrationId: string
  igId: string
  pageId: string
  inboxId: string
  name: string
  username: string
}

/** A connected Instagram account of `type` through the fixture extension. */
export const connectInstagram = async (
  tenant: Tenant,
  type: "instagram" | "facebook",
  extra: {
    hasCapiScope?: boolean
    datasetId?: string | null
    capiTestEventCode?: string | null
    capiDisconnected?: boolean
    tokenRefreshError?: string | null
  } = {},
): Promise<IgAccount> => {
  const igId = uniqueDigits()
  const pageId = uniqueDigits()
  const name = uniqueName("IG Account")
  const username = `parity_${uniqueId()}`
  const { integrationId, inboxId } = await (await ext()).createInstagramChannel({
    workspaceId: tenant.workspaceId,
    igId,
    pageId,
    name,
    username,
    type,
    ...extra,
  })
  return { integrationId, igId, pageId, inboxId, name, username }
}

// ---------------------------------------------------------------------------
// API channel + realtime (to watch automated replies)
// ---------------------------------------------------------------------------

export type ApiChannel = {
  inboxId: string
  channel: Api
  callbackHost: string
}

/**
 * An API channel whose outbound callback goes to a new `*.parity.test` host
 * on the fake upstream, which answers 200 `{"messageId": ...}` to every POST.
 */
export const createApiChannel = async (api: Api): Promise<ApiChannel> => {
  const callbackHost = uniqueHost("p110-cb")
  await fake.script({
    match: { host: callbackHost, method: "POST", path: "/hook" },
    respond: { status: 200, json: { messageId: `ext-${uniqueId()}` } },
  })
  const created = await api.post<{ connection: { inboxId: string }; secret: { token: string } }>("/v1/connections", {
    provider: "api",
    config: { name: uniqueName("p110").slice(0, 40), callbackUrl: `https://${callbackHost}/hook` },
  })
  expect(created.status, created.text).toBe(201)
  return {
    inboxId: created.json.connection.inboxId,
    channel: apiClient(created.json.secret.token),
    callbackHost,
  }
}

export type ApiContact = { contactId: string; sourceId: string; firstName: string }

/** Sends one inbound API-channel message from `sourceId`. */
export const sendInbound = async (
  channel: ApiChannel,
  contact: { sourceId: string; firstName: string },
  text: string,
): Promise<void> => {
  const sent = await channel.channel.post("/v1/channels/api/messages", {
    contact: { sourceId: contact.sourceId, firstName: contact.firstName, lastName: "Parity" },
    message: { sourceId: `msg-${uniqueId()}`, text },
  })
  expect(sent.status, sent.text).toBe(202)
}

/** An inbound API-channel message; waits until the worker made the contact. */
export const inboundContact = async (api: Api, channel: ApiChannel, text = "hello"): Promise<ApiContact> => {
  const sourceId = `ext-${uniqueId()}`
  const firstName = `Ada${uniqueId()}`
  await sendInbound(channel, { sourceId, firstName }, text)
  const contactId = await wait(async () => {
    const list = await api.get<{ data: Array<{ id: string; firstName: string }> }>("/v1/contacts")
    expect(list.status, list.text).toBe(200)
    const found = list.json.data.find((item) => item.firstName === firstName)
    expect(found, "contact from the inbound API message").toBeDefined()
    return (found as { id: string }).id
  })
  return { contactId, sourceId, firstName }
}

export type CallbackEnvelope = {
  event: string
  timestamp: string
  contact: { id: string; sourceId: string }
  message: { text: string; messageType: string; contentType: string; [key: string]: unknown }
}

/** Outbound callback records of the API channel since `since`. */
export const callbacks = async (channel: ApiChannel, since: number): Promise<UpstreamRecord[]> =>
  (await fake.requests({ kind: "http", host: channel.callbackHost, since })).filter(
    (record) => record.method === "POST" && record.path === "/hook",
  )

/** Texts of the outbound messages the API channel delivered since `since`. */
export const callbackTexts = async (channel: ApiChannel, since: number): Promise<string[]> =>
  (await callbacks(channel, since)).map((record) => parseBody<CallbackEnvelope>(record).message?.text)

/** A member realtime socket of the tenant's workspace (session cookie -> connect token). */
export const memberSocket = async (tenant: Tenant): Promise<{ socket: RealtimeSocket; http: HttpClient }> => {
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
  return { socket, http }
}
