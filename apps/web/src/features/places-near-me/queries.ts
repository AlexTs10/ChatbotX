// The places-near-me page reads (`placeCategoryService.list`,
// `placeLocationService.list`) in the Rust server (`POST /rpc/actions/<name>`).
import type {
  PlaceCategoryListItem,
  PlaceLocationItem,
} from "@chatbotx.io/business"
import { serverQuery } from "@/lib/server-action"

export const listPlaceCategories = (workspaceId: string) =>
  serverQuery<PlaceCategoryListItem[]>("listPlaceCategories", workspaceId, [
    { workspaceId },
  ])

export const listPlaceLocations = (input: {
  workspaceId: string
  name?: string
  page: number
  perPage: number
}) =>
  serverQuery<{ data: PlaceLocationItem[]; pageCount: number }>(
    "listPlaceLocations",
    input.workspaceId,
    [input],
  )
