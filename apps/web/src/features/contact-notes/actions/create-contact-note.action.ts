// Generated from apps/builder/src/features/contact-notes/actions/create-contact-note.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const createContactNoteAction = serverAction("createContactNoteAction", {
  workspace: true,
})
