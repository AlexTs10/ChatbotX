// Generated from apps/builder/src/features/leaderboards/actions/export-leaderboard.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const exportLeaderboardAction = serverAction("exportLeaderboardAction", { workspace: true, noInput: true })
export const getLeaderboardExportAction = serverAction("getLeaderboardExportAction", { workspace: true })
