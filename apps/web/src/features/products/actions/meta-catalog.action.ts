// Generated from apps/builder/src/features/products/actions/meta-catalog.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const connectMetaCatalogAction = serverAction(
  "connectMetaCatalogAction",
  { workspace: true, noInput: true },
)
export const getMetaCatalogStateAction = serverAction(
  "getMetaCatalogStateAction",
  { workspace: true, noInput: true },
)
export const selectMetaCatalogAction = serverAction("selectMetaCatalogAction", {
  workspace: true,
})
export const listMetaCatalogBusinessesAction = serverAction(
  "listMetaCatalogBusinessesAction",
  { workspace: true, noInput: true },
)
export const createMetaCatalogAction = serverAction("createMetaCatalogAction", {
  workspace: true,
})
export const syncToMetaCatalogAction = serverAction("syncToMetaCatalogAction", {
  workspace: true,
})
export const disconnectMetaCatalogAction = serverAction(
  "disconnectMetaCatalogAction",
  { workspace: true, noInput: true },
)
