// Generated from apps/builder/src/features/webhooks/actions/create-webhook-action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const createWebhookAction = serverAction("createWebhookAction", { workspace: true })
