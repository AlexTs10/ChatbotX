// Generated from apps/builder/src/features/discount-codes/actions/discount-code.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const createDiscountCodeAction = serverAction("createDiscountCodeAction", { workspace: true })
export const updateDiscountCodeAction = serverAction("updateDiscountCodeAction", { workspace: true })
export const setDiscountCodeActiveAction = serverAction("setDiscountCodeActiveAction", { workspace: true })
export const deleteDiscountCodeAction = serverAction("deleteDiscountCodeAction", { workspace: true, noInput: true })
export const generateDiscountCodeAction = serverAction("generateDiscountCodeAction", { workspace: true, noInput: true })
