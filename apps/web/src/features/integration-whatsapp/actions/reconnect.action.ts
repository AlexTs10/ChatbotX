// Generated from apps/builder/src/features/integration-whatsapp/actions/reconnect.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction, serverFunction } from "@/lib/server-action"

export const persistReconnectAuthAndResubscribe = serverFunction("persistReconnectAuthAndResubscribe")
export const reconnectWhatsappAction = serverAction("reconnectWhatsappAction", { workspace: true })
