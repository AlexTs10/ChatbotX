// Shared helpers of unit P1.12 (miniApps, minigames, openHours, orders,
// paymentProviders). Not a test file. Copied in part from
// tests/p1-11-messages-messenger/helpers.ts and
// tests/p1-07-conversations-decisions/helpers.ts (units do not share files).
// Setup goes through public routes where one exists and the P1.12 fixture
// extension where none does; every assertion stays black-box.

import { expect } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import { loadFixtureExt } from "../../src/fixtures/ext.ts"
import type { P112Fixtures } from "../../src/fixtures/ext/p1-12.ts"
import { createTenant, fixtures, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, type HttpResponse } from "../../src/http.ts"
import { uniqueId } from "../../src/unique.ts"

export type Api = ReturnType<typeof apiClient>

export const ext = (): Promise<P112Fixtures> => loadFixtureExt<P112Fixtures>("p1-12")

export const GRAPH = "graph.facebook.com"
export const STRIPE = "api.stripe.com"

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

/** Form-encoded body (Stripe) as a flat key -> value map (`metadata[orderId]` keys stay as they are). */
export const parseForm = (record: UpstreamRecord): Record<string, string> =>
  Object.fromEntries(new URLSearchParams(record.bodyText).entries())

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
  input: { firstName: string; lastName?: string; email?: string },
): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/contacts", {
    channel: "webchat",
    inboxId,
    firstName: input.firstName,
    lastName: input.lastName ?? "Parity",
    email: input.email ?? "",
    gender: "unknown",
  })
  expect(response.status, response.text).toBe(200)
  return response.json.id
}

/** A webchat inbox plus one contact on it; returns the contact id. */
export const newContact = async (api: Api, firstName = uniqueName("Buyer")): Promise<string> => {
  const { inboxId } = await createWebchat(api, uniqueName("Shop chat"))
  return await createWebchatContact(api, inboxId, { firstName })
}

/** A product made with the public `products.create` (setup only). */
export const createProduct = async (
  api: Api,
  input: { name?: string; price: number; inventoryPolicy?: "track" | "dont_track"; inventoryQuantity?: number },
): Promise<{ id: string; name: string }> => {
  const name = input.name ?? uniqueName("Product")
  const response = await api.post<{ id: string }>("/v1/products", {
    name,
    price: input.price,
    ...(input.inventoryPolicy ? { inventoryPolicy: input.inventoryPolicy } : {}),
    ...(input.inventoryQuantity !== undefined ? { inventoryQuantity: input.inventoryQuantity } : {}),
  })
  expect(response.status, response.text).toBe(200)
  return { id: response.json.id, name }
}
