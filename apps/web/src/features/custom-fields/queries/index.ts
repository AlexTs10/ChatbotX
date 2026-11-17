// apps/builder/src/features/custom-fields/queries for the SPA: the same read
// through the Rust `/rpc` (P4 spec §5).
import { client } from "@/lib/orpc/orpc"
import type {
  ListCustomFieldsRequest,
  ListCustomFieldsResponse,
} from "../schema/query"

export const listCustomFields = (
  input: ListCustomFieldsRequest & { workspaceId: string },
): Promise<ListCustomFieldsResponse> =>
  client.customFieldsAPI.privateListCustomFieldsAPI(input)

export const listCustomFieldsRSC = listCustomFields
