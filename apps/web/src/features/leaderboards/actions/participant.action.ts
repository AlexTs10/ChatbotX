// Generated from apps/builder/src/features/leaderboards/actions/participant.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const adjustParticipantScoreAction = serverAction("adjustParticipantScoreAction", { workspace: true })
export const setParticipantBannedAction = serverAction("setParticipantBannedAction", { workspace: true })
export const resetParticipantScoreAction = serverAction("resetParticipantScoreAction", { workspace: true })
export const getParticipantHistoryAction = serverAction("getParticipantHistoryAction", { workspace: true })
export const listLeaderboardAuditLogAction = serverAction("listLeaderboardAuditLogAction", { workspace: true })
