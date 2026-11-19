// Generated from apps/builder/src/features/webshop/actions/webshop.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
// The result types are the builder's inferred ones (see lib/server-action.ts).
import { serverAction } from "@/lib/server-action"
import type {
  ShopActionError,
  ShopActionResult,
} from "../lib/shop-action-types"

export const changeCartAction =
  serverAction<ShopActionResult<{ itemCount: number }>>("changeCartAction")
export const checkoutAction = serverAction<
  ShopActionResult<{
    orderId: string | null
    total: number
    currency: string
    paymentUrl: string | null
  }>
>("checkoutAction")
export const payOrderAction =
  serverAction<ShopActionResult<{ paymentUrl: string }>>("payOrderAction")
export const rateOrderAction = serverAction<
  ShopActionResult<{ rating: number | null; ratingComment: string | null }>
>("rateOrderAction")
export const applyDiscountCodeAction = serverAction<
  ShopActionResult<{ discountError: ShopActionError | null }>
>("applyDiscountCodeAction")
export const removeDiscountCodeAction = serverAction<
  ShopActionResult<{ removed: true }>
>("removeDiscountCodeAction")
