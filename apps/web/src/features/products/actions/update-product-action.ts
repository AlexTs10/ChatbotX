// Generated from apps/builder/src/features/products/actions/update-product-action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction, serverFunction } from "@/lib/server-action"

export const updateProductAction = serverAction("updateProductAction", {
  workspace: true,
})
export const updateProduct = serverFunction("updateProduct")
