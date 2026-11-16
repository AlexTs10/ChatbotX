// Generated from apps/builder/src/features/templates/actions/template-install-intent.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const rememberTemplateInstallIntentAction = serverAction(
  "rememberTemplateInstallIntentAction",
)
export const clearTemplateInstallIntentAction = serverAction(
  "clearTemplateInstallIntentAction",
  { noInput: true },
)
