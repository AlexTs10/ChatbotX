// apps/builder/src/features/folders/queries for the SPA: the same reads
// through the Rust `/rpc` (P4 spec §5).
import { client } from "@/lib/orpc/orpc"
import { loadPage } from "@/lib/server-action"
import type {
  GetCurrentFolderSchema,
  ListFoldersSearchParams,
} from "../schema/query"
import type { FolderResource, ListFoldersResponse } from "../schema/resource"

export const listFolders = (
  input: ListFoldersSearchParams & { isTrash?: boolean | null },
): Promise<{ data: ListFoldersResponse["data"] }> =>
  client.foldersAPI.listFoldersAuthenticatedAPI({
    ...input,
    folderId: input.folderId ?? null,
  })

export const getCurrentFolder = (
  input: GetCurrentFolderSchema,
): Promise<{ folder: FolderResource | null; parents: FolderResource[] }> =>
  loadPage("folder", input)
