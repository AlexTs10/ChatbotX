// apps/builder/src/features/sequences/queries for the SPA: the same read
// through the Rust `/rpc` (P4 spec §5).
import { client } from "@/lib/orpc/orpc"
import type {
  ListSequencesRequest,
  ListSequencesResponse,
} from "../schema/action"

export const listSequences = (
  input: ListSequencesRequest,
): Promise<ListSequencesResponse> =>
  client.sequencesAPI.listSequencesWorkspaceAuthAPI(input)
