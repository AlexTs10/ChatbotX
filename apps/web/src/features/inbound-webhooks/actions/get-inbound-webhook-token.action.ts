// Generated from apps/builder/src/features/inbound-webhooks/actions/get-inbound-webhook-token.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const getInboundWebhookTokenAction = serverAction("getInboundWebhookTokenAction", { workspace: true, noInput: true })
