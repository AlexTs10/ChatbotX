// Generated from apps/builder/src/features/integration-whatsapp/calling/actions/initiate-outbound-voip-call.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

// biome-ignore lint/suspicious/noExplicitAny: server type, not ported
export type InitiateOutboundVoipCallResult = any
export const initiateOutboundVoipCallAction = serverAction(
  "initiateOutboundVoipCallAction",
  { workspace: true },
)
