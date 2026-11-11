// Shared helpers of the P1.9 igComments / igStories / googleAds / inboxTeams
// tests (files 3*, 4*, 5*). Copied in part from
// tests/p1-05-channels-connections/helpers.ts (units do not share files).
// Setup goes through public routes where one exists and the P1.9 fixture
// extension where none does; every assertion stays black-box.

import { expect } from "vitest"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import { loadFixtureExt } from "../../src/fixtures/ext.ts"
import type { P109Fixtures } from "../../src/fixtures/ext/p1-09.ts"
import { createTenant, fixtures, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, type HttpResponse } from "../../src/http.ts"
import { uniqueId } from "../../src/unique.ts"

export const ext = (): Promise<P109Fixtures> => loadFixtureExt<P109Fixtures>("p1-09")

export type Api = ReturnType<typeof apiClient>

export const tenantApi = async (
  label: string,
  options: Parameters<typeof createTenant>[0] = {},
): Promise<{ tenant: Tenant; api: Api }> => {
  const tenant = await createTenant({ label, ...options })
  return { tenant, api: apiClient(tenant.token) }
}

/** Status, oRPC error `code` and (optionally) `defined` / `message` of an error response. */
export const expectError = (
  response: HttpResponse,
  status: number,
  code: string,
  options: { defined?: boolean; message?: string } = {},
): Record<string, unknown> => {
  const body = (response.json ?? {}) as Record<string, unknown>
  expect({ status: response.status, code: body.code }, response.text).toEqual({ status, code })
  expect(body.status).toBe(status)
  if (options.defined !== undefined) {
    expect(body.defined).toBe(options.defined)
  }
  if (options.message !== undefined) {
    expect(body.message).toBe(options.message)
  }
  return body
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

/** 422 `invalidRequestData` with an issue at `path` (dotted). */
export const expectInvalid = (response: HttpResponse, path: string): void => {
  expectError(response, 422, "invalidRequestData", { defined: true })
  expect(issuePaths(response)).toContain(path)
}

/** 422 business validation (`validationException`). */
export const expectValidation = (response: HttpResponse, message?: string): void => {
  expectError(response, 422, "validation", message === undefined ? {} : { message })
}

export const uniqueName = (prefix: string): string => `${prefix}-${uniqueId()}`

let numericCounter = 0
/** A unique all-digit id (Instagram account / page / customer ids are numeric). */
export const uniqueDigits = (): string => {
  numericCounter = (numericCounter + 1) % 1000
  return `${Date.now()}${String(numericCounter).padStart(3, "0")}${Math.floor(Math.random() * 1000)
    .toString()
    .padStart(3, "0")}`
}

/** A numeric id that no row has (far future snowflake range). */
export const UNKNOWN_ID = "999999999999999999"

/** Graph API path regex that accepts any `vNN.N` version prefix. */
export const graphPath = (rest: string): string => `^/v[0-9.]+/${rest}$`

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

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

export type IgAccount = { igId: string; pageId: string; inboxId: string; name: string; username: string }

/** A connected Instagram account of `type` ("instagram" = Instagram Login, "facebook" = via Facebook Login). */
export const connectInstagram = async (
  tenant: Tenant,
  type: "instagram" | "facebook",
  label = "ig",
): Promise<IgAccount> => {
  const igId = uniqueDigits()
  const pageId = uniqueDigits()
  const name = uniqueName(`${label}-account`)
  const username = `parity_${uniqueId()}`
  const { inboxId } = await (await ext()).createInstagramChannel({
    workspaceId: tenant.workspaceId,
    igId,
    pageId,
    name,
    username,
    type,
  })
  return { igId, pageId, inboxId, name, username }
}

/** A flow of the tenant's workspace (draft, no content). */
export const createFlow = async (api: Api, name = uniqueName("flow")): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/flows", { name, folderId: null })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

/** A second user who is an agent member of the tenant's workspace. */
export const addAgent = async (tenant: Tenant, label = "agent"): Promise<{ userId: string; email: string }> => {
  const driver = await fixtures()
  const email = `${label}-${uniqueId()}@parity.test`
  const { userId } = await driver.createUser({ email, password: "Parity#12345", name: `Parity ${label}` })
  await (await ext()).addWorkspaceMember({ workspaceId: tenant.workspaceId, userId })
  return { userId, email }
}

/** A user who is not a member of any workspace. */
export const createOutsider = async (label = "outsider"): Promise<{ userId: string }> => {
  const driver = await fixtures()
  return driver.createUser({ email: `${label}-${uniqueId()}@parity.test`, password: "Parity#12345", name: "Outsider" })
}

// ---------------------------------------------------------------------------
// Comment automation bodies
// ---------------------------------------------------------------------------

export const DEFAULT_OPTIONS = {
  replyToNewContactsOnly: false,
  replyOncePerUserPerPost: false,
  likeUserComment: false,
  replyToUsersWhoCommentedOnOtherPosts: true,
  ignoreCommentReplies: true,
  trackUserTags: false,
}

export const DEFAULT_HIDE = {
  all: false,
  hasPhoneNumber: false,
  hasImage: false,
  hasVideo: false,
  hasLink: false,
  hasKeywords: false,
  keywords: [] as string[],
  showCommentsAfter: "none",
}

/** A full `igComments.create` body. */
export const igCommentBody = (overrides: Record<string, unknown> = {}): Record<string, unknown> => ({
  name: uniqueName("igc"),
  type: "instagram",
  post: { type: "all", value: [] },
  privateReply: { type: "text", value: "Thanks for your comment" },
  publicReply: { type: "none", value: null },
  includeKeywords: { type: "all", value: [] },
  excludeKeywords: [],
  options: DEFAULT_OPTIONS,
  hideComments: DEFAULT_HIDE,
  replyAfter: { type: "immediately", value: 0 },
  ...overrides,
})

/** Keys of `igCommentPublicResource` (CommentAutomation columns minus workspaceId). */
export const IG_COMMENT_KEYS = [
  "clickedCount",
  "createdAt",
  "deliveredCount",
  "endTime",
  "excludeKeywords",
  "excludeKeywordsType",
  "failedCount",
  "folderId",
  "hideComments",
  "id",
  "inboxId",
  "includeKeywords",
  "isActive",
  "missedCount",
  "moderatedCount",
  "name",
  "options",
  "post",
  "privateReply",
  "publicReply",
  "repliesCount",
  "replyAfter",
  "seenCount",
  "sentCount",
  "startTime",
  "type",
  "updatedAt",
]

/** Keys of `igStoryPublicResource` (IgStoryAutomation columns minus workspaceId). */
export const IG_STORY_KEYS = [
  "createdAt",
  "endTime",
  "folderId",
  "id",
  "inboxId",
  "includeKeywords",
  "isActive",
  "name",
  "repliesCount",
  "reply",
  "startTime",
  "story",
  "trigger",
  "type",
  "updatedAt",
]

export const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/
export const SNOWFLAKE = /^\d{15,20}$/
