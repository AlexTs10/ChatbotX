// Generated from apps/builder/src/features/mini-apps/actions/publish-mini-app-whatsapp.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

/** `toPublishResponse` (builder `features/mini-apps/api/mappers.ts`). */
export const publishMiniAppWhatsappAction = serverAction<{
  status: string
  published: boolean
  whatsappFlowId: string | null
  validationErrors: unknown[]
}>("publishMiniAppWhatsappAction", { workspace: true })
