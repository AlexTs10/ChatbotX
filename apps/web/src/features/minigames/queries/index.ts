// apps/builder/src/features/minigames/queries for the SPA:
// `minigameService.list` in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type {
  ListMinigamesRequest,
  ListMinigamesResponse,
} from "../schema/query"

export const listMinigames = (input: ListMinigamesRequest) =>
  serverQuery<ListMinigamesResponse>("listMinigames", input.workspaceId, [
    input,
  ])
