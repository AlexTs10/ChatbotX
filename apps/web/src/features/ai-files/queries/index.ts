// apps/builder/src/features/ai-files/queries for the SPA: the same reads
// through the Rust `/rpc` (P4 spec §5).
import { ORPCError } from "@orpc/client"
import { client } from "@/lib/orpc/orpc"
import type { AIFileWithProcessing } from "@/lib/server-types"

export const listAIFiles = (input: {
  workspaceId: string
}): Promise<{ data: AIFileWithProcessing[] }> =>
  client.aiFilesAPI.listAIFilesAuthenticatedAPI({
    workspaceId: input.workspaceId,
  })

/** `null` when the file does not exist (the page shows its not-found). */
export const getAIFile = (input: {
  workspaceId: string
  id: string
}): Promise<AIFileWithProcessing | null> =>
  client.aiFilesAPI.getAIFileAuthenticatedAPI(input).catch((error) => {
    if (error instanceof ORPCError && error.status === 404) {
      return null
    }
    throw error
  })
