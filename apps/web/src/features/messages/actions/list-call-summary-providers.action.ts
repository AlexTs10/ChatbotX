// Generated from apps/builder/src/features/messages/actions/list-call-summary-providers.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const listCallSummaryProvidersAction = serverAction(
  "listCallSummaryProvidersAction",
  { workspace: true, noInput: true },
)
