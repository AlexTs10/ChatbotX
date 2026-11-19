// The client side of apps/builder/src/features/messages/actions/create-webchat-message.action.ts:
// the Rust server serves `handleCreateWebchatMessage` at `POST /api/guest/messages`
// (crates/server/src/webchat.rs), with the same checks (access token, origin,
// rate limit).
import type {
  AttachmentModel,
  MessageModel,
} from "@chatbotx.io/database/types"
import type { SafeActionResult } from "next-safe-action"
import type { z } from "zod"
import type { createWebchatMessageRequest } from "../schema/mutation"

type Schema = typeof createWebchatMessageRequest
/** `handleCreateWebchatMessage`'s `newMessage` (attachments with their URL). */
type WebchatMessage = MessageModel & {
  attachments: (AttachmentModel & { url: string })[]
}

const DEFAULT_SERVER_ERROR_MESSAGE =
  "Something went wrong while executing the operation."

export async function createWebchatMessageAction(
  input: z.input<Schema>,
): Promise<SafeActionResult<string, Schema, string, WebchatMessage | null>> {
  // ponytail: the JSON route has no file upload; the guest file picker
  // needs a multipart `/api/guest/messages` on the Rust side.
  if ("files" in input && Array.isArray(input.files) && input.files.length > 0) {
    return { serverError: DEFAULT_SERVER_ERROR_MESSAGE }
  }
  try {
    const response = await fetch("/api/guest/messages", {
      body: JSON.stringify(input),
      headers: { "Content-Type": "application/json" },
      method: "POST",
    })
    const body = (await response.json().catch(() => null)) as {
      data?: unknown
      message?: string
    } | null
    if (!response.ok) {
      return { serverError: body?.message ?? DEFAULT_SERVER_ERROR_MESSAGE }
    }
    return { data: (body?.data ?? null) as WebchatMessage | null }
  } catch {
    return { serverError: DEFAULT_SERVER_ERROR_MESSAGE }
  }
}
