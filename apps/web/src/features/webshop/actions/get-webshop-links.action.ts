// Generated from apps/builder/src/features/webshop/actions/get-webshop-links.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const getWebshopLinksAction = serverAction<{
  previewUrl: string
  buttonUrl: string
}>("getWebshopLinksAction", { workspace: true })
