// apps/builder/src/features/dynamic-images/queries for the SPA:
// `dynamicImageService.list` / `.find` in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type {
  ListDynamicImagesRequest,
  ListDynamicImagesResponse,
} from "../schema/query"
import type { DynamicImageResource } from "../schema/resource"

export const listDynamicImages = (input: ListDynamicImagesRequest) =>
  serverQuery<ListDynamicImagesResponse>("listDynamicImages", input.workspaceId, [
    input,
  ])

/** `findDynamicImage`: `null` for "not found". */
export const findDynamicImage = (where: { workspaceId: string; id: string }) =>
  serverQuery<DynamicImageResource | null>("findDynamicImage", where.workspaceId, [
    where,
  ])
