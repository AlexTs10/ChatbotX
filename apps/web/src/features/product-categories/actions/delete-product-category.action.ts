// Generated from apps/builder/src/features/product-categories/actions/delete-product-category.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const deleteProductCategoryAction = serverAction(
  "deleteProductCategoryAction",
  { workspace: true, noInput: true },
)
