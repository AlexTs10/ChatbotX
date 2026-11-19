// Generated from apps/builder/src/features/magic-links/actions/update-magic-link.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction, serverFunction } from "@/lib/server-action"

export const updateMagicLinkAction = serverAction("updateMagicLinkAction", { workspace: true })
export const updateMagicLink = serverFunction("updateMagicLink")
