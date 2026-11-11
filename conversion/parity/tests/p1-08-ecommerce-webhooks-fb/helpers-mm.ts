// Helpers of the P1.8 facebookMarketingMessages tests (files 30-33). Not a
// test file. The Marketing Messages grant, campaigns, audiences and
// recipients are fixture rows (they come from an OAuth step and the delivery
// worker); every assertion is HTTP or a fake upstream record.

import { expect } from "vitest"
import { fake, type UpstreamRecord } from "../../src/fake.ts"
import type { MmAudienceInput, MmCampaignInput } from "../../src/fixtures/ext/p1-08.ts"
import { uniqueId } from "../../src/unique.ts"
import { type Api, ext, GRAPH, graphPath, reEscape, script, tenantApi, uniqueDigits, upstreamCalls } from "./helpers.ts"

export const MM_SCOPE = "marketing_messages_messenger"

export const CONNECT_FIRST = "Connect Marketing Messages in the ChatbotX app first."
export const NOT_FOUND_MESSAGE = "Marketing message not found."

/** Keys of `campaignResourceSchema` (features/facebook-marketing-messages/schema/public.ts). */
export const CAMPAIGN_KEYS = [
  "adAccountId",
  "audienceId",
  "budgetMinorUnits",
  "budgetType",
  "campaignId",
  "completedAt",
  "content",
  "createdAt",
  "currency",
  "firstSentAt",
  "id",
  "name",
  "nextDayAt",
  "pageId",
  "startedAt",
  "startsAt",
  "status",
  "statusErrorCode",
  "statusErrorMessage",
  "statusReason",
  "targetType",
  "tokenCount",
]

export const AUDIENCE_KEYS = [
  "adAccountId",
  "createdAt",
  "customAudienceId",
  "description",
  "id",
  "name",
  "pageId",
  "readyAt",
  "rowsUploaded",
]

export const uniqueAct = (): string => `act_${uniqueDigits()}`

export const textContent = (text = "Parity promo") => ({ templateType: "text", text, quickReplies: [] })

export type Campaign = {
  id: string
  name: string
  pageId: string
  adAccountId: string
  currency: string
  budgetType: "daily" | "lifetime"
  budgetMinorUnits: number
  content: unknown
  targetType: string
  audienceId: string | null
  campaignId: string
  status: string
  statusReason: string | null
  statusErrorCode: number | null
  statusErrorMessage: string | null
  startsAt: string | null
  startedAt: string | null
  firstSentAt: string | null
  completedAt: string | null
  nextDayAt: string | null
  tokenCount: number | null
  createdAt: string
  stats?: unknown
}

export type MmTenant = Awaited<ReturnType<typeof tenantApi>> & {
  /** The grant's user access token (sent to Graph). */
  token: string
  appId: string
  appSecret: string
}

/**
 * A tenant with a usable Marketing Messages grant: the owner's Messenger app
 * credential, an active grant row and a scripted `/debug_token` that reports
 * the token valid with the Marketing Messages scope.
 */
export const mmTenant = async (
  label: string,
  options: { grant?: "active" | "invalid" | "expired" | "none"; scriptDebugToken?: boolean } = {},
): Promise<MmTenant> => {
  const base = await tenantApi(label)
  const fixtures = await ext()
  const appId = `parity-mm-app-${uniqueId()}`
  const appSecret = `parity-mm-secret-${uniqueId()}`
  const token = `mm-user-token-${uniqueId()}`
  await fixtures.createMessengerAppCredential({
    userId: base.tenant.userId,
    clientId: appId,
    clientSecret: appSecret,
    version: "v23.0",
  })
  const grant = options.grant ?? "active"
  if (grant !== "none") {
    await fixtures.createMarketingMessagesAuth({
      workspaceId: base.tenant.workspaceId,
      accessToken: token,
      status: grant === "invalid" ? "invalid" : "active",
      tokenExpiresAt: grant === "expired" ? new Date(Date.now() - 60_000) : new Date(Date.now() + 30 * 86_400_000),
      facebookUserId: `fbu-${uniqueDigits()}`,
    })
  }
  if (options.scriptDebugToken !== false) {
    await scriptDebugToken({ isValid: true, scopes: [MM_SCOPE, "ads_management"] })
  }
  return { ...base, token, appId, appSecret }
}

/** `/debug_token` answer (Graph path is shared by every test: newest script wins). */
export const scriptDebugToken = (input: { isValid?: boolean; scopes?: string[]; status?: number; times?: number }) =>
  script({
    match: { host: GRAPH, method: "GET", pathRegex: graphPath("debug_token") },
    respond:
      input.status && input.status >= 400
        ? { status: input.status, json: { error: { message: "Invalid OAuth access token.", type: "OAuthException", code: 190 } } }
        : { status: 200, json: { data: { is_valid: input.isValid ?? true, scopes: input.scopes ?? [MM_SCOPE] } } },
    ...(input.times ? { times: input.times } : {}),
  })

export type GraphAdAccount = {
  id: string
  account_id: string
  name?: string
  currency: string
  timezone_name?: string
  tos_accepted?: Record<string, number>
}

export const graphAdAccount = (
  id: string,
  options: { currency?: string; name?: string | null; tos?: boolean; timezone?: string } = {},
): GraphAdAccount => ({
  id,
  account_id: id.replace(/^act_/, ""),
  ...(options.name === null ? {} : { name: options.name ?? `Account ${id}` }),
  currency: options.currency ?? "USD",
  ...(options.timezone ? { timezone_name: options.timezone } : {}),
  tos_accepted: options.tos === false ? {} : { custom_audience_tos: 1 },
})

/** `GET /me/adaccounts` (shared path: newest script wins). */
export const scriptAdAccounts = (accounts: GraphAdAccount[], options: { status?: number; times?: number } = {}) =>
  script({
    match: { host: GRAPH, method: "GET", pathRegex: graphPath("me/adaccounts") },
    respond:
      options.status && options.status >= 400
        ? { status: options.status, json: graphError(100, "Unsupported get request.") }
        : { status: 200, json: { data: accounts } },
    ...(options.times ? { times: options.times } : {}),
  })

export const graphError = (code: number, message: string, subcode?: number) => ({
  error: { message, type: "OAuthException", code, ...(subcode === undefined ? {} : { error_subcode: subcode }), fbtrace_id: "parity" },
})

/** Graph records on one exact path (any version prefix), after `since`. */
export const graphCalls = (since: number, rest: string, method?: string): Promise<UpstreamRecord[]> =>
  upstreamCalls({ host: GRAPH, since, path: new RegExp(graphPath(reEscape(rest))), method })

/** All Graph records after `since` whose access token (query or bearer) is `token`. */
export const graphCallsWithToken = async (since: number, token: string): Promise<UpstreamRecord[]> =>
  (await fake.requests({ kind: "http", host: GRAPH, since })).filter(
    (record) =>
      record.query.access_token === token ||
      record.query.input_token === token ||
      record.headers.authorization === `Bearer ${token}` ||
      record.bodyText.includes(token),
  )

export const jsonBody = (record: UpstreamRecord | undefined): Record<string, unknown> =>
  JSON.parse(record?.bodyText || "{}") as Record<string, unknown>

/** A campaign row with sensible defaults (status `notSent`, USD daily 10.00). */
export const insertCampaign = async (
  workspaceId: string,
  input: Partial<Omit<MmCampaignInput, "workspaceId">> = {},
): Promise<{ id: string; campaignId: string; pageId: string; adAccountId: string }> => {
  const pageId = input.pageId ?? uniqueDigits()
  const adAccountId = input.adAccountId ?? uniqueAct()
  const campaignId = input.campaignId ?? uniqueDigits()
  const { id } = await (await ext()).createMarketingMessage({
    workspaceId,
    name: input.name ?? `Campaign ${uniqueId()}`,
    pageId,
    adAccountId,
    currency: input.currency ?? "USD",
    currencyOffset: input.currencyOffset ?? 100,
    budgetType: input.budgetType ?? "daily",
    budgetMinorUnits: input.budgetMinorUnits ?? 1000,
    content: input.content ?? textContent(),
    campaignId,
    status: input.status ?? "notSent",
    ...input,
  })
  return { id, campaignId, pageId, adAccountId }
}

export const insertAudience = async (
  workspaceId: string,
  input: Partial<Omit<MmAudienceInput, "workspaceId">> = {},
): Promise<{ id: string; customAudienceId: string; adAccountId: string; pageId: string }> => {
  const customAudienceId = input.customAudienceId ?? uniqueDigits()
  const adAccountId = input.adAccountId ?? uniqueAct()
  const pageId = input.pageId ?? uniqueDigits()
  const { id } = await (await ext()).createMarketingMessageAudience({
    workspaceId,
    name: input.name ?? `Audience ${uniqueId()}`,
    rowsUploaded: input.rowsUploaded ?? 0,
    readyAt: input.readyAt === undefined ? null : input.readyAt,
    ...input,
    customAudienceId,
    adAccountId,
    pageId,
  })
  return { id, customAudienceId, adAccountId, pageId }
}

export const getCampaign = async (api: Api, id: string): Promise<Campaign> => {
  const response = await api.get<Campaign>(`/v1/marketing-messages/campaigns/${id}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

/** ms between an ISO time and now (positive = in the future). */
export const msFromNow = (iso: string | null): number => (iso ? Date.parse(iso) - Date.now() : Number.NaN)

/** `YYYY-MM-DD` of `now` in a time zone. */
export const dayIn = (timeZone: string, date = new Date()): string =>
  new Intl.DateTimeFormat("en-CA", { timeZone, year: "numeric", month: "2-digit", day: "2-digit" }).format(date)

/** `YYYY-MM-DD` that is `days` calendar days before `day`. */
export const minusDays = (day: string, days: number): string => {
  const date = new Date(`${day}T00:00:00.000Z`)
  date.setUTCDate(date.getUTCDate() - days)
  return date.toISOString().slice(0, 10)
}
