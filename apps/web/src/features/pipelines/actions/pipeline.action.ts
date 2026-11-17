// Generated from apps/builder/src/features/pipelines/actions/pipeline.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const createPipelineAction = serverAction("createPipelineAction", {
  workspace: true,
})
export const updatePipelineAction = serverAction("updatePipelineAction", {
  workspace: true,
})
export const duplicatePipelineAction = serverAction("duplicatePipelineAction", {
  workspace: true,
})
export const deletePipelineAction = serverAction("deletePipelineAction", {
  workspace: true,
})
