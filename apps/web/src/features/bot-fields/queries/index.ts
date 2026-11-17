// apps/builder/src/features/bot-fields/queries for the SPA: the same read
// through the Rust `/rpc` (P4 spec §5).
import { client } from "@/lib/orpc/orpc"
import type {
  ListBotFieldsRequest,
  ListBotFieldsResponse,
} from "../schema/query"

/**
 * Documented hard cap for the Account Fields card on `/custom-fields`: the
 * card is a single client-driven table, fetched once and searched/paginated
 * in the browser.
 */
export const ACCOUNT_FIELDS_HARD_CAP = 500

export const listBotFieldsRSC = (
  input: ListBotFieldsRequest & { workspaceId: string },
): Promise<ListBotFieldsResponse> =>
  client.botFieldAPIs.privateListBotFieldsAPI({
    ...input,
    // Stable name ordering (the card would reshuffle after each save).
    sort: input.sort ?? [{ id: "name", desc: false }],
  })
