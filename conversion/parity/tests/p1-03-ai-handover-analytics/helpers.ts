// Shared helpers of unit P1.3 (aiHandover, aiMcpServers, analytics).

import { expect } from "vitest"
import { loadFixtureExt } from "../../src/fixtures/ext.ts"
import type { P103Fixtures } from "../../src/fixtures/ext/p1-03.ts"
import { createTenant, fixtures, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, type HttpResponse } from "../../src/http.ts"
import { uniqueEmail, uniqueId } from "../../src/unique.ts"

export const ext = (): Promise<P103Fixtures> => loadFixtureExt<P103Fixtures>("p1-03")

let syntheticCounter = 0n
/**
 * A unique decimal id for analytics rows whose contact / conversation /
 * message id has no foreign key (the event tables are scoped by workspace).
 */
export const syntheticId = (): string => {
  syntheticCounter += 1n
  return (BigInt(Date.now()) * 100_000n + (syntheticCounter % 100_000n)).toString()
}

export type Api = ReturnType<typeof apiClient>

export const tenantApi = async (
  label: string,
  options: Parameters<typeof createTenant>[0] = {},
): Promise<{ tenant: Tenant; api: Api }> => {
  const tenant = await createTenant({ label, ...options })
  return { tenant, api: apiClient(tenant.token) }
}

/** A second user who is an agent member of the workspace. */
export const addAgent = async (
  workspaceId: string,
  name: string,
): Promise<{ userId: string; email: string; name: string }> => {
  const driver = await fixtures()
  const email = uniqueEmail("agent")
  const { userId } = await driver.createUser({ email, password: "Parity#12345", name })
  await (await ext()).addMember({ workspaceId, userId })
  return { userId, email, name }
}

/** Status and oRPC error `code` (and `defined`) of an error response. */
export const expectError = (
  response: HttpResponse,
  status: number,
  code: string,
  options: { defined?: boolean } = {},
): void => {
  expect({ status: response.status, code: (response.json as { code?: string } | undefined)?.code }).toEqual({
    status,
    code,
  })
  const body = response.json as Record<string, unknown>
  expect(body.status).toBe(status)
  if (options.defined !== undefined) {
    expect(body.defined).toBe(options.defined)
  }
}

/** Issue paths of a 422 `invalidRequestData` error, each as a dotted string. */
export const issuePaths = (response: HttpResponse): string[] => {
  const data = (response.json as { data?: { issues?: Array<{ path?: unknown[] }> } }).data
  return (data?.issues ?? []).map((issue) =>
    (issue.path ?? []).map((segment) => (segment && typeof segment === "object" ? (segment as { key: unknown }).key : segment)).join("."),
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

/** UTC midnight ISO of a `YYYY-MM-DD` day (the builder runs with TZ=UTC). */
export const day = (key: string): string => `${key}T00:00:00.000Z`

export const uniqueName = (prefix: string): string => `${prefix}-${uniqueId()}`

export const sortBy = <T>(list: T[], key: (item: T) => string): T[] =>
  [...list].sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : 0))
