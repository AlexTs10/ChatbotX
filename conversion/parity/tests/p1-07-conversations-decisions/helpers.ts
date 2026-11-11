// Shared helpers of unit P1.7 (public API namespaces conversations, coupons,
// customFields, decisionConnections, decisionProfiles, discountCodes). Not a
// test file. Setup goes through public routes where one exists (webchats,
// contacts, tags, custom fields, flows) and through the P1.7 fixture
// extension where none does. Every assertion stays black-box.

import { expect } from "vitest"
import { loadFixtureExt } from "../../src/fixtures/ext.ts"
import type { P107Fixtures } from "../../src/fixtures/ext/p1-07.ts"
import { createTenant, fixtures, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, type HttpResponse } from "../../src/http.ts"
import { comparableError } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"

export type Api = ReturnType<typeof apiClient>

export const ext = (): Promise<P107Fixtures> => loadFixtureExt<P107Fixtures>("p1-07")

// The public API allows 120 requests per 10 s per workspace (429 after that).
// Each client keeps itself under 100 per 10 s per workspace. This only spaces
// requests out; it waits for no effect of the stack.
const WINDOW_MS = 10_000
const MAX_PER_WINDOW = 100
const sent = new Map<string, number[]>()
const throttle = async (key: string): Promise<void> => {
  const log = sent.get(key) ?? []
  sent.set(key, log)
  for (;;) {
    const now = Date.now()
    while (log.length > 0 && (log[0] as number) <= now - WINDOW_MS) {
      log.shift()
    }
    if (log.length < MAX_PER_WINDOW) {
      log.push(now)
      return
    }
    await new Promise((resolve) => setTimeout(resolve, (log[0] as number) + WINDOW_MS - now + 5))
  }
}

/** `apiClient(token)` that keeps under the per-workspace rate limit of `key`. */
export const limitedApi = (token: string, key: string): Api => {
  const api = apiClient(token)
  const wrap =
    <F extends (...args: never[]) => Promise<unknown>>(fn: F) =>
    async (...args: Parameters<F>) => {
      await throttle(key)
      return (await fn(...args)) as Awaited<ReturnType<F>>
    }
  return {
    http: api.http,
    get: wrap(api.get) as Api["get"],
    post: wrap(api.post) as Api["post"],
    put: wrap(api.put) as Api["put"],
    patch: wrap(api.patch) as Api["patch"],
    delete: wrap(api.delete) as Api["delete"],
  }
}

/** A new user + workspace + full token, and an API client for it. */
export const newTenant = async (
  label: string,
  options: Parameters<typeof createTenant>[0] = {},
): Promise<{ tenant: Tenant; api: Api }> => {
  const tenant = await createTenant({ label, ...options })
  return { tenant, api: limitedApi(tenant.token, tenant.workspaceId) }
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

export type ErrorBody = {
  defined?: boolean
  code?: string
  status?: number
  message?: string
  data?: { issues?: Array<{ path?: Array<string | number | { key: string | number }> }> } & Record<string, unknown>
}

/** Asserts HTTP status + oRPC error code (+ `defined` / `message` when given); returns the body. */
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

/** Status, code and `defined` of an error (P1.1 rule 9). */
export const errorShape = (response: HttpResponse<unknown>) => {
  const { data: _data, ...rest } = comparableError(response.json)
  return { httpStatus: response.status, ...rest }
}

/** Dotted paths of the zod issues of a 422 `invalidRequestData`, sorted. */
export const issuePaths = (response: HttpResponse<unknown>): string[] => {
  const body = (response.json ?? {}) as ErrorBody
  return (body.data?.issues ?? [])
    .map((issue) =>
      (issue.path ?? []).map((segment) => (segment && typeof segment === "object" ? segment.key : segment)).join("."),
    )
    .sort()
}

/** Asserts a 422 `invalidRequestData` whose issues include `path`. */
export const expectInvalid = (response: HttpResponse<unknown>, path: string): void => {
  expectError(response, 422, "invalidRequestData", { defined: true })
  expect(issuePaths(response), response.text).toContain(path)
}

/**
 * Query string. Arrays use oRPC bracket notation (`key[]=a&key[]=b`): a
 * plain single `key=a` is a string, not a one-item array.
 */
export const qs = (params: Record<string, string | number | boolean | string[] | undefined | null>): string => {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null) {
      continue
    }
    if (Array.isArray(value)) {
      for (const item of value) {
        search.append(`${key}[]`, item)
      }
    } else {
      search.set(key, String(value))
    }
  }
  const text = search.toString()
  return text ? `?${text}` : ""
}

export const uniqueName = (prefix: string): string => `${prefix}-${uniqueId()}`

export const sortBy = <T>(list: T[], key: (item: T) => string): T[] =>
  [...list].sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : 0))

/** A snowflake-shaped id that no row has. */
export const UNKNOWN_ID = "1000000000000000001"

/** ISO string check (the value is a date-time the stack wrote). */
export const expectIsoNear = (value: unknown, testStart: number, slackMs = 5_000): void => {
  expect(typeof value, String(value)).toBe("string")
  const time = Date.parse(value as string)
  expect(Number.isNaN(time), String(value)).toBe(false)
  expect(time, String(value)).toBeGreaterThanOrEqual(testStart - slackMs)
  expect(time, String(value)).toBeLessThanOrEqual(Date.now() + slackMs)
}

// ---------------------------------------------------------------------------
// Setup through public routes of other namespaces (not parity assertions here)
// ---------------------------------------------------------------------------

export type Webchat = { id: string; inboxId: string }

/** A webchat channel (Inbox channel "webchat" + IntegrationWebchat). */
export const createWebchat = async (api: Api, name = "Parity webchat"): Promise<Webchat> => {
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
  input: { firstName: string; lastName?: string; email?: string; phoneNumber?: string },
): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/contacts", {
    channel: "webchat",
    inboxId,
    firstName: input.firstName,
    lastName: input.lastName ?? "Parity",
    email: input.email ?? "",
    ...(input.phoneNumber ? { phoneNumber: input.phoneNumber } : {}),
    gender: "unknown",
  })
  expect(response.status, response.text).toBe(200)
  return response.json.id
}

export const createTag = async (api: Api, name: string): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/tags", { name })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

/** A published flow with one text step. */
export const createPublishedFlow = async (api: Api, name: string, text = "Parity flow"): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/flows", {
    name,
    folderId: null,
    spec: { formatVersion: 1, name, steps: [{ type: "send", text }] },
    publish: true,
  })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

export const idOf = (contactId: string): string => `id:${contactId}`

/** URL-encoded identifier path segment. */
export const contactPath = (identifier: string, suffix = ""): string =>
  `/v1/contacts/${encodeURIComponent(identifier)}${suffix}`
