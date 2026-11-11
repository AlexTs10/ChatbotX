// Shared helpers of unit P1.5 (channels, commentAutomations, connectSessions,
// connectionProviders, connections, contactScans). Copied in part from
// tests/p1-03-ai-handover-analytics/helpers.ts (units do not share files).

import { expect } from "vitest"
import { fake, type ScriptInput, type UpstreamRecord } from "../../src/fake.ts"
import { loadFixtureExt } from "../../src/fixtures/ext.ts"
import type { P105Fixtures } from "../../src/fixtures/ext/p1-05.ts"
import { createTenant, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, HttpClient, type HttpResponse } from "../../src/http.ts"
import { uniqueId } from "../../src/unique.ts"

export const ext = (): Promise<P105Fixtures> => loadFixtureExt<P105Fixtures>("p1-05")

export type Api = ReturnType<typeof apiClient>

export const tenantApi = async (
  label: string,
  options: Parameters<typeof createTenant>[0] = {},
): Promise<{ tenant: Tenant; api: Api }> => {
  const tenant = await createTenant({ label, ...options })
  return { tenant, api: apiClient(tenant.token) }
}

/** Status, oRPC error `code` and (optionally) `defined` of an error response. */
export const expectError = (
  response: HttpResponse,
  status: number,
  code: string,
  options: { defined?: boolean; message?: string } = {},
): void => {
  const body = (response.json ?? {}) as Record<string, unknown>
  expect({ status: response.status, code: body.code }).toEqual({ status, code })
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

/**
 * Issue paths of a 422 error whose `data` is either `{issues}` (oRPC input
 * validation) or a bare zod issue array (a `ZodError` thrown in a handler).
 */
export const anyIssuePaths = (response: HttpResponse): string[] => {
  const data = (response.json as { data?: unknown }).data
  const issues = Array.isArray(data) ? data : ((data as { issues?: unknown[] } | undefined)?.issues ?? [])
  return (issues as Array<{ path?: unknown[] }>).map((issue) =>
    (issue.path ?? [])
      .map((segment) => (segment && typeof segment === "object" ? (segment as { key: unknown }).key : segment))
      .join("."),
  )
}

/** 422 `invalidRequestData` with an issue at `path` (dotted). */
export const expectInvalid = (response: HttpResponse, path: string): void => {
  expectError(response, 422, "invalidRequestData", { defined: true })
  expect(issuePaths(response)).toContain(path)
}

/** Query string with repeated keys for arrays. */
export const qs = (params: Record<string, string | number | boolean | string[] | undefined>): string => {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined) {
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

export const uniqueName = (prefix: string): string => `${prefix}-${uniqueId()}`

let numericCounter = 0
/** A unique all-digit id (Facebook page / IG account / Telegram bot ids are numeric). */
export const uniqueDigits = (): string => {
  numericCounter = (numericCounter + 1) % 1000
  return `${Date.now()}${String(numericCounter).padStart(3, "0")}${Math.floor(Math.random() * 1000)
    .toString()
    .padStart(3, "0")}`
}

/** ISO time `ms` from now (negative = in the past). */
export const isoFromNow = (ms: number): string => new Date(Date.now() + ms).toISOString()

/** Escape a string for a `pathRegex` script match. */
export const reEscape = (value: string): string => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")

/** Graph API path regex that accepts any `vNN.N` version prefix. */
export const graphPath = (rest: string): string => `^/v[0-9.]+/${rest}$`

export const script = (...scripts: ScriptInput[]) => fake.script(...scripts)

/** HTTP records of the fake upstream for one host, after `since`, filtered by path regex. */
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

/** A plain client of the app (no token), for routes outside `/api`. */
export const appClient = (): HttpClient => new HttpClient()

/** Shape check of a `ConnectionResource` (`features/connections/schema/resource.ts`). */
export const CONNECTION_KEYS = [
  "authExpiresAt",
  "capabilities",
  "channel",
  "connectedAt",
  "createdAt",
  "disconnectedAt",
  "displayName",
  "id",
  "inboxId",
  "integrationId",
  "kind",
  "lastError",
  "provider",
  "sourceId",
  "status",
  "statusReason",
  "strategy",
  "updatedAt",
]

export const SESSION_KEYS = [
  "connectionIds",
  "errorCode",
  "expiresAt",
  "id",
  "nextAction",
  "provider",
  "purpose",
  "status",
  "step",
  "targets",
]
