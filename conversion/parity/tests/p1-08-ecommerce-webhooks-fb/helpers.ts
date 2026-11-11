// Shared helpers of unit P1.8 (dynamicImages, ecommerceAnalytics,
// ecommerceSettings, emailTopics, errorLogs, externalWebhooks,
// facebookLeadAds, facebookMarketingMessages, fbComments). Not a test file.
// Copied in part from tests/p1-04 and tests/p1-05 helpers (units do not
// share files). Setup goes through public routes where one exists and the
// P1.8 fixture extension where none does; every assertion stays black-box.

import { expect } from "vitest"
import { fake, type ScriptInput, type UpstreamRecord } from "../../src/fake.ts"
import type { P108Fixtures } from "../../src/fixtures/ext/p1-08.ts"
import { loadFixtureExt } from "../../src/fixtures/ext.ts"
import { createTenant, fixtures, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, HttpClient, type HttpResponse } from "../../src/http.ts"
import { comparableError } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"

export const ext = (): Promise<P108Fixtures> => loadFixtureExt<P108Fixtures>("p1-08")

export type Api = ReturnType<typeof apiClient>

export const GRAPH = "graph.facebook.com"

/** A new user + workspace + full token, and an API client for it. */
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

type ErrorBody = {
  defined?: boolean
  code?: string
  status?: number
  message?: string
  data?: unknown
}

/** Asserts HTTP status + oRPC `code`/`status` (and optionally `defined`, `message`). */
export const expectError = (
  response: HttpResponse,
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
export const errorShape = (response: HttpResponse) => {
  const { data: _data, ...rest } = comparableError(response.json)
  return { httpStatus: response.status, ...rest }
}

/**
 * Issue paths of a 422 error whose `data` is either `{issues}` (oRPC input
 * validation) or a bare zod issue array (a `ZodError` thrown in a handler).
 */
export const issuePaths = (response: HttpResponse): string[] => {
  const data = (response.json as { data?: unknown } | undefined)?.data
  const issues = Array.isArray(data) ? data : ((data as { issues?: unknown[] } | undefined)?.issues ?? [])
  return (issues as Array<{ path?: unknown[] }>)
    .map((issue) =>
      (issue.path ?? [])
        .map((segment) => (segment && typeof segment === "object" ? (segment as { key: unknown }).key : segment))
        .join("."),
    )
    .sort()
}

/** 422 `invalidRequestData` (defined) with an issue at `path` (dotted). */
export const expectInvalid = (response: HttpResponse, path: string): void => {
  expectError(response, 422, "invalidRequestData", { defined: true })
  expect(issuePaths(response), response.text).toContain(path)
}

/** Query string with repeated keys for arrays. */
export const qs = (params: Record<string, string | number | boolean | string[] | undefined | null>): string => {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null) {
      continue
    }
    if (Array.isArray(value)) {
      for (const item of value) {
        search.append(key, item)
      }
    } else {
      search.set(key, String(value))
    }
  }
  const text = search.toString()
  return text ? `?${text}` : ""
}

/** Query string in oRPC bracket notation for arrays of objects: `sort[0][id]=name&sort[0][desc]=false`. */
export const sortQuery = (sort: Array<{ id: string; desc: boolean }>): string =>
  sort
    .flatMap((entry, index) => [
      `${encodeURIComponent(`sort[${index}][id]`)}=${encodeURIComponent(entry.id)}`,
      `${encodeURIComponent(`sort[${index}][desc]`)}=${entry.desc}`,
    ])
    .join("&")

export const uniqueName = (prefix: string): string => `${prefix}-${uniqueId()}`

let numericCounter = 0
/** A unique all-digit id (Facebook page / ad account / campaign ids are numeric). */
export const uniqueDigits = (): string => {
  numericCounter = (numericCounter + 1) % 1000
  return `${Date.now()}${String(numericCounter).padStart(3, "0")}${Math.floor(Math.random() * 1000)
    .toString()
    .padStart(3, "0")}`
}

/** Escape a string for a `pathRegex` script match. */
export const reEscape = (value: string): string => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")

/** Graph API path regex that accepts any `vNN.N` version prefix. */
export const graphPath = (rest: string): string => `^/v[0-9.]+/${rest}$`

export const script = (...scripts: ScriptInput[]) => fake.script(...scripts)

/** HTTP records of the fake upstream for one host, after `since`, filtered by path regex. */
export const upstreamCalls = async (input: {
  host: string
  since?: number
  path?: RegExp
  method?: string
}): Promise<UpstreamRecord[]> => {
  const records = await fake.requests({ kind: "http", host: input.host, since: input.since })
  return records.filter(
    (record) =>
      (!input.path || input.path.test(record.path)) && (!input.method || record.method === input.method.toUpperCase()),
  )
}

/** DoH lookups (`GET https://1.1.1.1/dns-query?name=<host>`) the SSRF guard made for `host`. */
export const dohLookups = async (host: string, since?: number): Promise<UpstreamRecord[]> =>
  (await fake.requests({ kind: "http", host: "1.1.1.1", pathPrefix: "/dns-query", since })).filter(
    (record) => record.query.name === host,
  )

/** A plain client of the app (no token), for routes outside `/api`. */
export const appClient = (): HttpClient => new HttpClient()

/** Sorted keys of an object, for exact shape checks. */
export const keysOf = (value: unknown): string[] => Object.keys((value ?? {}) as object).sort()

/** A connected Messenger Page (fixture rows) with a unique numeric page id. */
export const messengerPage = async (workspaceId: string, name?: string) => {
  const pageId = uniqueDigits()
  const pageName = name ?? `Page ${pageId}`
  const channel = await (await ext()).createMessengerChannel({ workspaceId, pageId, name: pageName })
  return { pageId, name: pageName, accessToken: channel.accessToken, ...channel }
}
