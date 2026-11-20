// Generated from apps/builder/src/lib/auth/utils.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverFunction } from "@/lib/server-action"

// biome-ignore lint/suspicious/noExplicitAny: server type, not ported
export type SessionUser = any
export const assertCurrentUserCanAccessChatbot = serverFunction(
  "assertCurrentUserCanAccessChatbot",
)
