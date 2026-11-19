// apps/builder/src/features/spreadsheets/queries for the SPA: the list runs in
// the Rust server (`listSpreadsheets`, P4 spec §5.1).
import type { PaginatedResponse } from "@/features/common/schema/pagination"
import { serverQuery } from "@/lib/server-action"
import type { ListSpreadsheetsRequest } from "../schema/query"
import type { SpreadsheetResource } from "../schema/resource"

export const listSpreadsheets = (
  input: ListSpreadsheetsRequest,
): Promise<PaginatedResponse<SpreadsheetResource>> =>
  serverQuery<PaginatedResponse<SpreadsheetResource>>(
    "listSpreadsheets",
    input.workspaceId,
    [input],
  )
