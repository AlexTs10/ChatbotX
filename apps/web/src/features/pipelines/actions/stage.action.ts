// Generated from apps/builder/src/features/pipelines/actions/stage.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const createStageAction = serverAction("createStageAction", { workspace: true })
export const updateStageAction = serverAction("updateStageAction", { workspace: true })
export const deleteStageAction = serverAction("deleteStageAction", { workspace: true })
export const deleteStageMovingItemsAction = serverAction("deleteStageMovingItemsAction", { workspace: true })
