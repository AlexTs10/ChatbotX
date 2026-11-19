// Generated from apps/builder/src/features/leaderboards/actions/delete-leaderboard.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const deleteLeaderboardAction = serverAction("deleteLeaderboardAction", { workspace: true, noInput: true })
