// apps/builder/src/features/import/queries/list-imports.queries.ts for the
// SPA: the Rust `/rpc` page query (`rpc_procedures/contact_tools.rs`).
import { serverQuery } from "@/lib/server-action"
import type { ListImportsRequest, ListImportsResponse } from "../schema/query"

export const listImports = (
  input: ListImportsRequest & { workspaceId: string },
): Promise<ListImportsResponse> =>
  serverQuery("listImports", input.workspaceId, [input.workspaceId, input])
