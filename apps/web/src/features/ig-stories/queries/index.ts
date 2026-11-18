// apps/builder/src/features/ig-stories/queries for the SPA: the public
// `igStories.list` / `igStories.get` routes in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type {
  ListIgStoriesRequest,
  ListIgStoriesResponse,
} from "../schema/action"
import type { IgStoryResource } from "../schema/resource"

export const listIgStories = (input: ListIgStoriesRequest) =>
  serverQuery<ListIgStoriesResponse>("listIgStories", input.workspaceId, [input])

export const getIgStory = (workspaceId: string, id: string) =>
  serverQuery<IgStoryResource>("getIgStory", workspaceId, [{ id }])
