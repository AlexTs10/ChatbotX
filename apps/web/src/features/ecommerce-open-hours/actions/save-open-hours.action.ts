// Generated from apps/builder/src/features/ecommerce-open-hours/actions/save-open-hours.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const createOpenHoursAction = serverAction("createOpenHoursAction", {
  workspace: true,
})
export const updateOpenHoursAction = serverAction("updateOpenHoursAction", {
  workspace: true,
})
export const deleteOpenHoursAction = serverAction("deleteOpenHoursAction", {
  workspace: true,
  noInput: true,
})
