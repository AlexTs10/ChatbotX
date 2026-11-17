// `listUserPersistentMenus` / `findUserPersistentMenu` (builder
// `features/user-persistent-menus/queries`) in the Rust server.
import { serverQuery } from "@/lib/server-action"
import type { ListUserPersistentMenusResponse } from "../schema/action"
import type { UserPersistentMenuResource } from "../schema/resource"

export const listUserPersistentMenus = (input: { workspaceId: string }) =>
  serverQuery<ListUserPersistentMenusResponse>(
    "listUserPersistentMenus",
    input.workspaceId,
    [input],
  )

/** `null` = not found (the builder page's `notFound()`). */
export const findUserPersistentMenu = (input: {
  id: string
  workspaceId: string
}) =>
  serverQuery<UserPersistentMenuResource | null>(
    "findUserPersistentMenu",
    input.workspaceId,
    [input],
  )
