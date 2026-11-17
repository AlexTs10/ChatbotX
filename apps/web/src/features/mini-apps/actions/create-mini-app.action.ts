// Generated from apps/builder/src/features/mini-apps/actions/create-mini-app.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const createMiniAppAction = serverAction("createMiniAppAction", { workspace: true })
