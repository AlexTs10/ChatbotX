// Generated from apps/builder/src/features/shared/comment-automation/actions/process-missed-comments.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const processMissedCommentsAction = serverAction("processMissedCommentsAction", { workspace: true, noInput: true })
