// Ported from apps/builder (server component): the records come from the
// Rust page loader `pages.deletedWorkspaces` (P4 spec §5.2).
import { useQuery } from "@tanstack/react-query"
import { useTranslations } from "next-intl"
import { loadPage } from "@/lib/server-action"
import type { WorkspaceDeletionRecordResource } from "../schema/deletion-record"
import { DeletionRecordEvidence } from "./deletion-record-evidence"

export const DeletedWorkspaces = ({ ownerId }: { ownerId: string }) => {
  const t = useTranslations()
  const { data: records } = useQuery({
    queryKey: ["spa", "deletedWorkspaces", ownerId],
    queryFn: () =>
      loadPage<{ data: WorkspaceDeletionRecordResource[] }>(
        "deletedWorkspaces",
        {
          page: 1,
          perPage: 20,
        },
      ),
  })
  if (!records || records.data.length === 0) {
    return null
  }

  return (
    <section className="space-y-2">
      <h2 className="font-semibold text-base">
        {t("platformAdmin.workspaces.deletedTitle")}
      </h2>
      <ul className="space-y-2">
        {records.data.map((record) => (
          <li className="rounded-md border p-3 text-sm" key={record.id}>
            <div className="font-medium">{record.workspaceName}</div>
            <DeletionRecordEvidence record={record} />
          </li>
        ))}
      </ul>
    </section>
  )
}
