// apps/builder/src/features/import/queries/list-imports.queries.ts for the
// SPA: the pages get the import history from their page query
// (`productsPage`); this module keeps the type the components import.
import type { ListImportsRequest, ListImportsResponse } from "../schema/query"

export declare function listImports(
  input: ListImportsRequest & { workspaceId: string },
): Promise<ListImportsResponse>
