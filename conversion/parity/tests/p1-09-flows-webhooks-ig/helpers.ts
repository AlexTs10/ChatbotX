// Shared helpers of unit P1.9, part A (flows, folders, auth matrix). Not a
// test file. Parts B (inbound webhooks) and C (ig/googleAds/teams) keep their
// own helper files. Copied in part from tests/p1-04-appointments-broadcasts
// and tests/p1-05-channels-connections (units do not share files).

import { expect } from "vitest"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import { loadFixtureExt } from "../../src/fixtures/ext.ts"
import type { P109Fixtures } from "../../src/fixtures/ext/p1-09.ts"
import { createTenant, fixtures, signedInClient, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, type HttpClient, type HttpResponse } from "../../src/http.ts"
import { connectRealtime, type RealtimeSocket, workspaceSocketUrl } from "../../src/realtime.ts"
import { rpc } from "../../src/rpc.ts"
import { uniqueHost, uniqueId } from "../../src/unique.ts"

export type Api = ReturnType<typeof apiClient>

export const ext = (): Promise<P109Fixtures> => loadFixtureExt<P109Fixtures>("p1-09")

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
  expectError(response, 422, "invalidRequestData")
  expect(issuePaths(response), response.text).toContain(path)
}

export type AuthoringError = {
  path: string
  code: string
  message: string
  hint?: string
  candidates?: string[]
  capability?: Record<string, unknown>
}

/** 422 `invalidRequestData` whose `data` is the flow-authoring error list. */
export const expectAuthoringErrors = (response: HttpResponse<unknown>): AuthoringError[] => {
  const body = expectError(response, 422, "invalidRequestData")
  expect(Array.isArray(body.data), response.text).toBe(true)
  return body.data as AuthoringError[]
}

/** Query string with repeated keys for arrays. */
export const qs = (params: Record<string, string | number | boolean | undefined>): string => {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined) {
      search.set(key, String(value))
    }
  }
  const text = search.toString()
  return text ? `?${text}` : ""
}

// ---------------------------------------------------------------------------
// Flows
// ---------------------------------------------------------------------------

export type FlowSpec = {
  formatVersion: 1
  name: string
  channel?: string
  steps: Array<Record<string, unknown>>
}

export const textSpec = (name: string, ...texts: string[]): FlowSpec => ({
  formatVersion: 1,
  name,
  steps: texts.map((text) => ({ type: "send", text })),
})

export type FlowVersion = {
  id: string
  flowId: string
  workspaceId: string
  startNodeId: string
  isDraft: boolean
  isLatest: boolean
  nodes: Array<Record<string, unknown> & { id: string; type?: string; data?: Record<string, unknown> }>
  edges: Array<Record<string, unknown> & { id: string; source: string; target: string }>
  createdAt: string
}

export type FlowDetail = {
  id: string
  name: string
  workspaceId: string
  folderId: string | null
  active: boolean
  enableInInbox: boolean
  currentVersionId: string | null
  draftVersionId: string | null
  createdAt: string
  updatedAt: string
  flowVersions: FlowVersion[]
  welcomeMessageFlows: unknown[]
}

/** POST /v1/flows; asserts 201 and returns the new id. */
export const createFlow = async (api: Api, body: Record<string, unknown>): Promise<{ id: string; nodeIds?: Record<string, string> }> => {
  const response = await api.post<{ id: string; nodeIds?: Record<string, string> }>("/v1/flows", {
    folderId: null,
    ...body,
  })
  expect(response.status, response.text).toBe(201)
  return response.json
}

/** A published flow made from a spec. */
export const createPublishedFlow = async (api: Api, spec: FlowSpec, extra: Record<string, unknown> = {}): Promise<string> =>
  (await createFlow(api, { name: spec.name, spec, publish: true, ...extra })).id

export const getFlow = async (api: Api, id: string): Promise<FlowDetail> => {
  const response = await api.get<FlowDetail>(`/v1/flows/${id}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

export const draftOf = (flow: FlowDetail): FlowVersion => {
  const draft = flow.flowVersions.find((version) => version.isDraft)
  expect(draft, JSON.stringify(flow)).toBeDefined()
  return draft as FlowVersion
}

type GraphStep = { stepType?: string; text?: string; [key: string]: unknown }

/** The steps of a node (`data.details.steps`). */
export const nodeSteps = (node: { data?: Record<string, unknown> }): GraphStep[] =>
  ((node.data?.details as { steps?: GraphStep[] } | undefined)?.steps ?? []) as GraphStep[]

/** Text bodies of the sendText steps of a graph, in node order. */
export const sendTexts = (version: Pick<FlowVersion, "nodes">): string[] =>
  version.nodes.flatMap((node) =>
    nodeSteps(node)
      .filter((step) => step.stepType === "sendText")
      .map((step) => step.text ?? ""),
  )

// ---------------------------------------------------------------------------
// API channel + realtime (to watch a flow run)
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
  const callbackHost = uniqueHost("p109-cb")
  await fake.script({
    match: { host: callbackHost, method: "POST", path: "/hook" },
    respond: { status: 200, json: { messageId: `ext-${uniqueId()}` } },
  })
  const created = await api.post<{ connection: { inboxId: string }; secret: { token: string } }>("/v1/connections", {
    provider: "api",
    config: { name: uniqueName("p109").slice(0, 40), callbackUrl: `https://${callbackHost}/hook` },
  })
  expect(created.status, created.text).toBe(201)
  return {
    inboxId: created.json.connection.inboxId,
    channel: apiClient(created.json.secret.token),
    callbackHost,
  }
}

export type ApiContact = { contactId: string; sourceId: string; firstName: string }

/** An inbound API-channel message; waits until the worker made the contact. */
export const inboundContact = async (
  api: Api,
  channel: ApiChannel,
  waitFor: <T>(fn: () => Promise<T>) => Promise<T>,
): Promise<ApiContact> => {
  const sourceId = `ext-${uniqueId()}`
  const firstName = `Ada${uniqueId()}`
  const sent = await channel.channel.post("/v1/channels/api/messages", {
    contact: { sourceId, firstName, lastName: "Parity" },
    message: { sourceId: `msg-${uniqueId()}`, text: "hello" },
  })
  expect(sent.status, sent.text).toBe(202)
  const contactId = await waitFor(async () => {
    const list = await api.get<{ data: Array<{ id: string; firstName: string }> }>("/v1/contacts")
    expect(list.status, list.text).toBe(200)
    const found = list.json.data.find((item) => item.firstName === firstName)
    expect(found, "contact from the inbound API message").toBeDefined()
    return (found as { id: string }).id
  })
  return { contactId, sourceId, firstName }
}

/** Outbound callback records of the API channel since `since`. */
export const callbacks = async (channel: ApiChannel, since: number): Promise<UpstreamRecord[]> =>
  (await fake.requests({ kind: "http", host: channel.callbackHost, since })).filter(
    (record) => record.method === "POST" && record.path === "/hook",
  )

export const parseBody = <T = Record<string, unknown>>(record: UpstreamRecord): T => JSON.parse(record.bodyText) as T

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
