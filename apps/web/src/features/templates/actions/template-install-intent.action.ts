// Generated from apps/builder/src/features/templates/actions/template-install-intent.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
// The marker cookie (`cookies().set` / `.delete` in the builder) is set here:
// it holds no secret, the choice itself stays server-side.
import { serverAction } from "@/lib/server-action"
import {
  INSTALL_INTENT_MARKER_COOKIE,
  INSTALL_INTENT_MARKER_MAX_AGE_SECONDS,
} from "../lib/install-intent-marker"

const remember = serverAction("rememberTemplateInstallIntentAction")
const clear = serverAction("clearTemplateInstallIntentAction", {
  noInput: true,
})

export const rememberTemplateInstallIntentAction: typeof remember = async (
  ...args
) => {
  const result = await remember(...args)
  if (!(result.serverError || result.validationErrors)) {
    // biome-ignore lint/suspicious/noDocumentCookie: a plain marker cookie, as the builder's
    document.cookie = `${INSTALL_INTENT_MARKER_COOKIE}=1; Max-Age=${INSTALL_INTENT_MARKER_MAX_AGE_SECONDS}; Path=/; SameSite=Lax`
  }
  return result
}

export const clearTemplateInstallIntentAction: typeof clear = async (
  ...args
) => {
  const result = await clear(...args)
  // biome-ignore lint/suspicious/noDocumentCookie: a plain marker cookie, as the builder's
  document.cookie = `${INSTALL_INTENT_MARKER_COOKIE}=; Max-Age=0; Path=/`
  return result
}
