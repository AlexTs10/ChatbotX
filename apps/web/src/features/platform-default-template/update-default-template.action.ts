// Generated from apps/builder/src/features/platform-default-template/update-default-template.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const updateDefaultTemplateAction = serverAction("updateDefaultTemplateAction")
export const clearDefaultTemplateAction = serverAction("clearDefaultTemplateAction", { noInput: true })
export const updateRootDefaultTemplateAction = serverAction("updateRootDefaultTemplateAction")
export const clearRootDefaultTemplateAction = serverAction("clearRootDefaultTemplateAction", { noInput: true })
