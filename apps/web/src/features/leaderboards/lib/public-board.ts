// The public leaderboard data (`/rpc/pages/leaderboardPublic` and the
// `/api/leaderboards/{id}/{board,me}` polling routes of the Rust server).
import type { getLeaderboardPublicBoard } from "@chatbotx.io/business/leaderboard"

export type PublicLeaderboardBoard = NonNullable<
  Awaited<ReturnType<typeof getLeaderboardPublicBoard>>
>

export type PublicLeaderboardViewer = {
  rank: number | null
  score: number
  name: string | null
  avatar: string | null
  shareUrl: string | null
}
