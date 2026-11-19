// apps/builder/src/features/leaderboards/queries for the SPA: the
// `leaderboardService` reads run in the Rust server (`POST /rpc/actions/<name>`,
// with the workspace membership check).
import type {
  LeaderboardParticipantFilter,
  leaderboardParticipantService,
} from "@chatbotx.io/business/leaderboard"
import type {
  LeaderboardPlatform,
  LeaderboardStatus,
} from "@chatbotx.io/database/partials"
import type { LeaderboardModel } from "@chatbotx.io/database/types"
import { serverQuery } from "@/lib/server-action"

/** `toLeaderboardResource`: the public API row (dates as ISO text). */
export type LeaderboardRow = Omit<
  LeaderboardModel,
  "workspaceId" | "deletedAt"
> & {
  stats: { participants: number; totalPoints: number; newToday: number }
  availableActions: string[]
  viewUrl: string
}

export type ListLeaderboardsResult = {
  data: LeaderboardRow[]
  pageCount: number
  total: number
  summary: Record<string, number>
}

/** List page: rows shaped like the public API (incl. `availableActions`, `viewUrl`). */
export const listLeaderboards = (input: {
  workspaceId: string
  status: LeaderboardStatus | null
  platform: LeaderboardPlatform | null
  keyword: string | null
  page: number
  perPage: number
}) =>
  serverQuery<ListLeaderboardsResult>("listLeaderboards", input.workspaceId, [
    input,
  ])

/** One leaderboard, or null when it is missing from this workspace. */
export const findLeaderboard = (input: { workspaceId: string; id: string }) =>
  serverQuery<LeaderboardRow | null>("findLeaderboard", input.workspaceId, [
    input,
  ])

export type ListLeaderboardParticipantsResult = Awaited<
  ReturnType<typeof leaderboardParticipantService.list>
>

export const listLeaderboardParticipants = (input: {
  workspaceId: string
  leaderboardId: string
  keyword: string | null
  filter: LeaderboardParticipantFilter
  page: number
  perPage: number
}) =>
  serverQuery<ListLeaderboardParticipantsResult>(
    "listLeaderboardParticipants",
    input.workspaceId,
    [input],
  )

export type LeaderboardWizardData = {
  minigameOptions: { label: string; value: string }[]
  /** `leaderboardService.defaultRules(platform)` for every platform. */
  defaultRules: Record<LeaderboardPlatform, LeaderboardModel["rules"]>
  /** `LEADERBOARD_PLATFORM_CHANNEL`. */
  platformChannels: Record<LeaderboardPlatform, string | null>
}

/** The create/edit wizard inputs (minigame picker, default rules, channels). */
export const getLeaderboardWizardData = (workspaceId: string) =>
  serverQuery<LeaderboardWizardData>("getLeaderboardWizardData", workspaceId, [
    { workspaceId },
  ])

/** Minigame picker options for the Gamification platform. */
export const listMinigameOptions = async (workspaceId: string) =>
  (await getLeaderboardWizardData(workspaceId)).minigameOptions
