// Generated from apps/builder/src/features/pipelines/actions/field-member.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const createFieldAction = serverAction("createFieldAction", { workspace: true })
export const updateFieldAction = serverAction("updateFieldAction", { workspace: true })
export const deleteFieldAction = serverAction("deleteFieldAction", { workspace: true })
export const setMemberAction = serverAction("setMemberAction", { workspace: true })
export const removeMemberAction = serverAction("removeMemberAction", { workspace: true })
