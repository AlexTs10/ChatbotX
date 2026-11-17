// Generated from apps/builder/src/features/pipelines/actions/item.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const createItemAction = serverAction("createItemAction", {
  workspace: true,
})
export const updateItemAction = serverAction("updateItemAction", {
  workspace: true,
})
export const moveItemAction = serverAction("moveItemAction", {
  workspace: true,
})
export const transferItemAction = serverAction("transferItemAction", {
  workspace: true,
})
export const deleteItemAction = serverAction("deleteItemAction", {
  workspace: true,
})
export const addCommentAction = serverAction("addCommentAction", {
  workspace: true,
})
export const deleteCommentAction = serverAction("deleteCommentAction", {
  workspace: true,
})
export const addAttachmentsAction = serverAction("addAttachmentsAction", {
  workspace: true,
})
export const removeAttachmentAction = serverAction("removeAttachmentAction", {
  workspace: true,
})
