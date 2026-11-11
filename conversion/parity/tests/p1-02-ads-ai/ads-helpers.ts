// Shared helpers of the P1.2 ads tests. Not a test file (no `.test.ts`).

import { randomInt } from "node:crypto"
import { expect } from "vitest"
import { fake, type ScriptInput, type UpstreamRecord } from "../../src/fake.ts"
import { loadFixtureExt } from "../../src/fixtures/ext.ts"
import type { MessagingAdChannel, P102Fixtures } from "../../src/fixtures/ext/p1-02.ts"
import { createTenant, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, type HttpResponse } from "../../src/http.ts"
import { comparableError } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"

export type Api = ReturnType<typeof apiClient>

export const GRAPH = "graph.facebook.com"
export const GRAPH_VERSION = "v23.0"
/** A snowflake-shaped id that no row has. */
export const MISSING_ID = "999999999999999999"
export const ADS_SCOPE_MESSAGE = "Token is not authorized for the 'ads' scope"
export const READ_ONLY_MESSAGE = "Read-only token cannot perform this operation"
export const RECONNECT_MESSAGE = "This connection needs to be reconnected."

// 1x1 transparent PNG.
export const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
  "base64",
)

export const adsFixtures = () => loadFixtureExt<P102Fixtures>("p1-02")

let digitCounter = randomInt(0, 1000)
/** Unique numeric string (Meta-style id), `length` digits (min 13, starts with 9). */
export const uniqueDigits = (length = 15): string => {
  digitCounter = (digitCounter + 1) % 1000
  const base = `9${String(Date.now()).slice(-9)}${String(digitCounter).padStart(3, "0")}`
  const pad = Array.from({ length: Math.max(0, length - base.length) }, () => randomInt(0, 10)).join("")
  return `${base}${pad}`
}

export const uniqueAdAccountId = () => `act_${uniqueDigits(15)}`
export const uniqueAccessToken = (label = "msgads") => `EAA-parity-${label}-${uniqueId()}`

export const expectError = (
  response: HttpResponse,
  status: number,
  code: string,
  message?: string,
) => {
  expect(response.status, response.text).toBe(status)
  expect(comparableError(response.json, { keepMessage: message !== undefined })).toMatchObject({
    code,
    status,
    ...(message !== undefined ? { message } : {}),
  })
}

/** 422 input validation error whose issues include one at `path`. */
export const expectValidationIssue = (response: HttpResponse, path: Array<string | number>) => {
  expectError(response, 422, "invalidRequestData")
  const issues = ((response.json as { data?: { issues?: Array<{ path?: unknown[] }> } }).data?.issues ?? []).map(
    (issue) => JSON.stringify(issue.path ?? []),
  )
  expect(issues).toContain(JSON.stringify(path))
}

export type MessengerAdsSetup = {
  tenant: Tenant
  api: Api
  integrationId: string
  inboxId: string
  pageId: string
  accessToken: string
}

/** Tenant + connected Messenger Page (+ an active messaging-ads connection unless `connect: false`). */
export const messengerAdsSetup = async (
  options: { label?: string; connect?: boolean; status?: "active" | "invalid" } = {},
): Promise<MessengerAdsSetup> => {
  const tenant = await createTenant({ label: options.label ?? "ads" })
  const ext = await adsFixtures()
  const pageId = uniqueDigits(15)
  const { integrationId, inboxId } = await ext.createMessengerChannel({
    workspaceId: tenant.workspaceId,
    pageId,
    name: `Parity Page ${pageId}`,
  })
  const accessToken = uniqueAccessToken()
  if (options.connect !== false) {
    await ext.connectMessagingAds({
      workspaceId: tenant.workspaceId,
      channel: "messenger",
      integrationId,
      accessToken,
      status: options.status,
    })
  }
  return { tenant, api: apiClient(tenant.token), integrationId, inboxId, pageId, accessToken }
}

export const graphScript = (
  method: string,
  path: string,
  respond: ScriptInput["respond"],
  extra: { times?: number; bodyIncludes?: string } = {},
) =>
  fake.script({
    match: {
      host: GRAPH,
      method,
      path: `/${GRAPH_VERSION}${path}`,
      ...(extra.bodyIncludes ? { bodyIncludes: extra.bodyIncludes } : {}),
    },
    respond,
    ...(extra.times !== undefined ? { times: extra.times } : {}),
  })

/** Graph error body as Meta sends it. */
export const graphError = (status: number, code: number, message: string) => ({
  status,
  json: { error: { message, type: "OAuthException", code, fbtrace_id: "parity" } },
})

/** Graph requests made with `accessToken` (query or JSON body) after `since`. */
export const graphCalls = async (accessToken: string, since = 0): Promise<UpstreamRecord[]> =>
  (await fake.requests({ kind: "http", host: GRAPH, since })).filter(
    (record) => record.query.access_token === accessToken || record.bodyText.includes(accessToken),
  )

export const jsonBody = (record: UpstreamRecord | undefined): Record<string, unknown> =>
  JSON.parse(record?.bodyText || "{}") as Record<string, unknown>

export const callSummary = (records: UpstreamRecord[]) =>
  records.map((record) => `${record.method} ${record.path}`)

export type { MessagingAdChannel }
