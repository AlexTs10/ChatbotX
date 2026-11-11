// Leaderboard helpers of unit P1.10 (files 5*). Not a test file.
// Host channels are fixture inboxes (zalo / telegram) with no integration
// behind them, so no scoring path reaches a third party; scores come from the
// manual adjust route.

import { expect } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { uniqueId } from "../../src/unique.ts"
import { type Api, ext, uniqueName } from "./helpers.ts"

export type Rule = {
  enabled: boolean
  points: number
  maxPerDay: number | null
  minChars: number | null
  bonusPoints: number | null
  bonusMinutes: number | null
}

export const rule = (overrides: Partial<Rule> = {}): Rule => ({
  enabled: false,
  points: 0,
  maxPerDay: null,
  minChars: null,
  bonusPoints: null,
  bonusMinutes: null,
  ...overrides,
})

export type Leaderboard = {
  id: string
  name: string
  description: string
  coverImageUrl: string
  startsAt: string
  endsAt: string
  status: "draft" | "active" | "paused" | "finished"
  platform: string
  inboxId: string | null
  minigameId: string | null
  scope: { postScope: string; postIds: string[]; telegramChatIds: string[] }
  rules: Record<string, Rule>
  followersOnly: boolean
  tagIds: string[]
  resetTime: string
  cooldownSeconds: number
  referralWelcomeFlowId: string | null
  rewardSettings: { flowId: string | null; topN: number; autoOnFinish: boolean }
  rewardsTriggeredAt: string | null
  finishedAt: string | null
  createdAt: string
  updatedAt: string
  stats: { participants: number; totalPoints: number; newToday: number }
  availableActions: string[]
  viewUrl: string
}

export const LEADERBOARD_KEYS = [
  "availableActions",
  "cooldownSeconds",
  "coverImageUrl",
  "createdAt",
  "description",
  "endsAt",
  "finishedAt",
  "followersOnly",
  "id",
  "inboxId",
  "minigameId",
  "name",
  "platform",
  "referralWelcomeFlowId",
  "resetTime",
  "rewardSettings",
  "rewardsTriggeredAt",
  "rules",
  "scope",
  "startsAt",
  "stats",
  "status",
  "tagIds",
  "updatedAt",
  "viewUrl",
]

export const ACTIONS = {
  draft: ["edit", "publish", "delete"],
  active: ["pause", "edit", "resetPoints", "finish", "export"],
  paused: ["start", "edit", "export", "delete"],
  finished: ["export", "triggerRewards", "delete"],
}

export const NOT_FOUND = "Leaderboard not found"

/** ISO time `offsetMs` from now, with millisecond precision. */
export const isoFromNow = (offsetMs: number): string => new Date(Date.now() + offsetMs).toISOString()

export const HOUR = 60 * 60 * 1000
export const DAY = 24 * HOUR

export const viewUrlOf = (id: string, token = "{{leaderboard_view_token}}") =>
  `http://app/leaderboards?leaderboardId=${id}&token=${token}`

/** A connected Zalo host inbox (fixture, no integration). */
export const zaloInbox = async (tenant: Tenant): Promise<string> =>
  (await (await ext()).createInbox({ workspaceId: tenant.workspaceId, channel: "zalo", name: uniqueName("Zalo OA") }))
    .inboxId

/** A minimal valid create body for a Zalo leaderboard (message rule on, no referral). */
export const zaloBody = (inboxId: string, overrides: Record<string, unknown> = {}): Record<string, unknown> => ({
  name: uniqueName("Board"),
  startsAt: isoFromNow(-HOUR),
  endsAt: isoFromNow(7 * DAY),
  platform: "zalo",
  inboxId,
  rules: { message: { enabled: true, points: 10 } },
  ...overrides,
})

export const createLeaderboard = async (api: Api, body: Record<string, unknown>): Promise<Leaderboard> => {
  const response = await api.post<Leaderboard>("/v1/leaderboards", body)
  expect(response.status, response.text).toBe(201)
  return response.json
}

export const getLeaderboard = async (api: Api, id: string): Promise<Leaderboard> => {
  const response = await api.get<Leaderboard>(`/v1/leaderboards/${id}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

export const transition = async (api: Api, id: string, action: "publish" | "pause" | "start" | "finish") => {
  const response = await api.post<Leaderboard>(`/v1/leaderboards/${id}/${action}`, {})
  expect(response.status, response.text).toBe(200)
  return response.json
}

/** An active Zalo leaderboard of the tenant. */
export const activeLeaderboard = async (
  api: Api,
  tenant: Tenant,
  overrides: Record<string, unknown> = {},
): Promise<Leaderboard> => {
  const inboxId = await zaloInbox(tenant)
  const created = await createLeaderboard(api, zaloBody(inboxId, overrides))
  return transition(api, created.id, "publish")
}

/** A contact of the tenant's workspace (fixture), with an optional phone. */
export const contact = async (
  tenant: Tenant,
  input: { firstName: string; lastName?: string; phoneNumber?: string | null; inboxId?: string; conversation?: boolean },
): Promise<string> =>
  (
    await (await ext()).createContact({
      workspaceId: tenant.workspaceId,
      firstName: input.firstName,
      lastName: input.lastName ?? "Parity",
      phoneNumber: input.phoneNumber ?? null,
      inboxes: input.inboxId ? [{ inboxId: input.inboxId, channel: "zalo", sourceId: `zl-${uniqueId()}` }] : [],
      conversation: input.conversation,
    })
  ).contactId

export const adjust = async (api: Api, leaderboardId: string, contactId: string, delta: number, reason = "manual"): Promise<number> => {
  const response = await api.post<{ score: number }>(
    `/v1/leaderboards/${leaderboardId}/participants/${contactId}/adjust`,
    { delta, reason },
  )
  expect(response.status, response.text).toBe(200)
  return response.json.score
}
