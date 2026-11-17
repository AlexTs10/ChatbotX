// Generated from apps/builder/src/features/integration-whatsapp/calling/actions/answer-voip-call.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

// biome-ignore lint/suspicious/noExplicitAny: server type, not ported
export type AnswerWhatsappVoipCallResult = any
export const answerWhatsappVoipCallAction = serverAction(
  "answerWhatsappVoipCallAction",
  { workspace: true },
)
