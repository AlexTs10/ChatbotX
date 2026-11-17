// apps/builder/src/features/mini-apps/queries for the SPA: the Rust server
// runs each query under its export name (P4 spec §5.1; Q.D1m).
import type { MiniAppModel } from "@chatbotx.io/database/types"
import { serverQuery } from "@/lib/server-action"
import type { ListMiniAppsRequest } from "../schema/query"

type MiniAppPublicationRow = {
  id: string
  integrationWhatsappId: string
  sourceId: string
  status: string
  publishedAt: Date | null
}

export type MiniAppListRow = Omit<MiniAppModel, "definition" | "flowJson"> & {
  publications: MiniAppPublicationRow[]
}

export const listMiniApps = (input: ListMiniAppsRequest) =>
  serverQuery<{ data: MiniAppListRow[]; pageCount: number }>(
    "listMiniApps",
    input.workspaceId,
    [input],
  )

/** `findMiniApp`: `null` for "not found". */
export const findMiniApp = (input: { workspaceId: string; id: string }) =>
  serverQuery<
    (MiniAppModel & { publications: MiniAppPublicationRow[] }) | null
  >("findMiniApp", input.workspaceId, [input])

export const listMiniAppSubmissions = (input: {
  workspaceId: string
  miniAppId: string
  page: number
  perPage: number
}) =>
  serverQuery<
    Awaited<
      ReturnType<
        typeof import("@chatbotx.io/business/mini-app").miniAppSubmissionService.list
      >
    >
  >("listMiniAppSubmissions", input.workspaceId, [input])

export type WhatsappPublishTarget = { id: string; label: string }

/** The workspace's WhatsApp numbers, reduced to what the publish picker shows. */
export const listWhatsappPublishTargets = (workspaceId: string) =>
  serverQuery<WhatsappPublishTarget[]>(
    "listWhatsappPublishTargets",
    workspaceId,
    [],
  )
