// Generated from apps/builder/src/features/integration-api/actions/rotate-token.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const rotateApiTokenAction = serverAction("rotateApiTokenAction", {
  workspace: true,
  noInput: true,
})
