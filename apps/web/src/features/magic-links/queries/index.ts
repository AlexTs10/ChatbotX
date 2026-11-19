// apps/builder/src/features/magic-links/queries for the SPA: the magic link
// reads run in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type {
  ListMagicLinksRequest,
  ListMagicLinksResponse,
} from "../schema/query"

export const listMagicLinks = (input: ListMagicLinksRequest) =>
  serverQuery<ListMagicLinksResponse>("listMagicLinks", input.workspaceId, [
    input,
  ])

/** `magicLinkService.findByWorkspace`: `null` when not in the workspace. */
export const findMagicLink = (input: { workspaceId: string; id: string }) =>
  serverQuery<{ id: string; name: string } | null>(
    "findMagicLink",
    input.workspaceId,
    [input],
  )
