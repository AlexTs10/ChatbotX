// Ported from apps/builder (server component): the folder bar loads through
// the Rust `/rpc` and reads the folder from the URL.
import { type FolderType, rootFolderId } from "@chatbotx.io/database/partials"
import type { FolderModel } from "@chatbotx.io/database/types"
import { Skeleton } from "@chatbotx.io/ui/components/ui/skeleton"
import { useSearchParams } from "next/navigation"
import { Suspense, useMemo } from "react"
import { ListFolders } from "@/features/folders/list-folders"
import { getCurrentFolder, listFolders } from "@/features/folders/queries"
import { listFoldersSearchParams } from "@/features/folders/schema/query"

export default function SharedFolderSlot(props: {
  folderType: FolderType
  workspaceId: string
}) {
  const { folderType, workspaceId } = props
  const searchParams = useSearchParams()
  const { folderId } = listFoldersSearchParams.parse(
    Object.fromEntries(searchParams),
  )

  const promises = useMemo(
    () =>
      Promise.all([
        folderId
          ? getCurrentFolder({ id: folderId, workspaceId })
          : Promise.resolve({ folder: null, parents: [] as FolderModel[] }),
        listFolders({
          workspaceId,
          folderType,
          folderId: folderId ?? rootFolderId,
        }),
      ]),
    [folderId, folderType, workspaceId],
  )

  return (
    <Suspense fallback={<Skeleton className="mb-4 h-8 w-full" />}>
      <ListFolders
        folderType={folderType}
        // Remount per folder so per-level UI state (show more) starts collapsed.
        key={folderId ?? rootFolderId}
        promises={promises}
        workspaceId={workspaceId}
      />
    </Suspense>
  )
}
