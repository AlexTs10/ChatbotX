// apps/builder/src/features/sequences/queries for the SPA: the same read
// through the Rust `/rpc` (P4 spec §5).
import type { SequenceEditor } from "@/features/sequences/sequence-editor"
import { client } from "@/lib/orpc/orpc"
import { serverQuery } from "@/lib/server-action"
import type {
  ListSequencesRequest,
  ListSequencesResponse,
} from "../schema/action"

export const listSequences = (
  input: ListSequencesRequest,
): Promise<ListSequencesResponse> =>
  client.sequencesAPI.listSequencesWorkspaceAuthAPI(input)

type SequenceWithSteps = Parameters<typeof SequenceEditor>[0]["sequence"]

/** `sequenceService.findWithSteps` (`null` = not found). */
export const getSequence = (workspaceId: string, sequenceId: string) =>
  serverQuery<SequenceWithSteps | null>("getSequence", workspaceId, [
    workspaceId,
    sequenceId,
  ])
