// The client-safe part of the builder module (the sync scope check); the
// operations run on the Rust server.
import type { MetaCatalogSyncScope } from "@chatbotx.io/database/partials"

export type MetaCatalogSyncInput = {
  scope: MetaCatalogSyncScope
  /**
   * Destination catalog; rebinds the connection when it differs from the
   * stored one. Omitted: the catalog the connection is bound to.
   */
  catalogId?: string
  categoryId?: string
  selectedProductIds?: string[]
}

const SYNC_SCOPE_VALIDATORS = {
  all: () => true,
  category: (input: MetaCatalogSyncInput) => Boolean(input.categoryId),
  selected: (input: MetaCatalogSyncInput) =>
    Boolean(input.selectedProductIds?.length),
} as const satisfies Record<
  MetaCatalogSyncScope,
  (input: MetaCatalogSyncInput) => boolean
>

export const isMetaCatalogSyncScopeComplete = (
  input: MetaCatalogSyncInput,
): boolean => SYNC_SCOPE_VALIDATORS[input.scope](input)
