// Generated from apps/builder/src/features/integration-whatsapp/verification/actions.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const requestWhatsappVerificationCodeAction = serverAction("requestWhatsappVerificationCodeAction", { workspace: true })
export const verifyWhatsappPhoneCodeAction = serverAction("verifyWhatsappPhoneCodeAction", { workspace: true })
