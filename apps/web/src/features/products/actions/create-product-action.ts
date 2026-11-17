// Generated from apps/builder/src/features/products/actions/create-product-action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction, serverFunction } from "@/lib/server-action"

export const createProductAction = serverAction("createProductAction", {
  workspace: true,
})
export const createProduct = serverFunction("createProduct")
