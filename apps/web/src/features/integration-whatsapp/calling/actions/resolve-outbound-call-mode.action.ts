// Generated from apps/builder/src/features/integration-whatsapp/calling/actions/resolve-outbound-call-mode.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

// biome-ignore lint/suspicious/noExplicitAny: server type, not ported
export type OutboundCallPermissionStatus = any
// biome-ignore lint/suspicious/noExplicitAny: server type, not ported
export type NoneCallModeReason = any
// biome-ignore lint/suspicious/noExplicitAny: server type, not ported
export type ResolveOutboundCallModeResult = any
export const resolveOutboundCallModeAction = serverAction(
  "resolveOutboundCallModeAction",
  { workspace: true },
)
