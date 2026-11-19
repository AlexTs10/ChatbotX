// Generated from apps/builder/src/features/inbound-webhooks/actions/get-inbound-webhook-request.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const getInboundWebhookRequestAction = serverAction("getInboundWebhookRequestAction", { workspace: true, noInput: true })
