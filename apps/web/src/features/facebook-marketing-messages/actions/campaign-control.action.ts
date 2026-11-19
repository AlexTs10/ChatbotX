// Generated from apps/builder/src/features/facebook-marketing-messages/actions/campaign-control.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const stopMarketingMessageAction = serverAction("stopMarketingMessageAction", { workspace: true, noInput: true })
export const resumeMarketingMessageAction = serverAction("resumeMarketingMessageAction", { workspace: true, noInput: true })
export const sendMarketingMessageAction = serverAction("sendMarketingMessageAction", { workspace: true, noInput: true })
