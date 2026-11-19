// The client part of the builder's server-only `queries`: the Rust page
// queries (`whatsappChannelPage`) return this non-secret subset; never the
// `auth` / `capiAccessToken` columns.
import type { IntegrationWhatsappModel } from "@chatbotx.io/database/types"

export type IntegrationWhatsappLinkable = Pick<
  IntegrationWhatsappModel,
  "id" | "workspaceId" | "businessId" | "wabaId"
>
