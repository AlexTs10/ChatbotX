// Generated from apps/builder/src/features/admin-workspaces/queries/index.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverFunction } from "@/lib/server-action"

// biome-ignore lint/suspicious/noExplicitAny: server type, not ported
export type ListAdminWorkspacesResponse = any
export const listAdminWorkspaces = serverFunction("listAdminWorkspaces")
