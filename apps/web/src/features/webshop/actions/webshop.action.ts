// Generated from apps/builder/src/features/webshop/actions/webshop.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const changeCartAction = serverAction("changeCartAction")
export const checkoutAction = serverAction("checkoutAction")
export const payOrderAction = serverAction("payOrderAction")
export const rateOrderAction = serverAction("rateOrderAction")
export const applyDiscountCodeAction = serverAction("applyDiscountCodeAction")
export const removeDiscountCodeAction = serverAction("removeDiscountCodeAction")
