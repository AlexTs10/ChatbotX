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

/** `findMinigame`: `null` for "not found". */
export const findMinigame = (where: { workspaceId: string; id: string }) =>
  serverQuery<import("../schema/resource").MinigameResource | null>(
    "findMinigame",
    where.workspaceId,
    [where],
  )

/** `listMinigameHistory` (`minigameContactService.list`). */
export const listMinigameHistory = (
  input: import("../schema/query").ListMinigameHistoryRequest,
) =>
  serverQuery<
    Awaited<
      ReturnType<
        typeof import("@chatbotx.io/business/minigame").minigameContactService.list
      >
    >
  >("listMinigameHistory", input.workspaceId, [input])
