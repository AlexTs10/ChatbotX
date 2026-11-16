// `integrationWhatsappResource` of `@chatbotx.io/business` (integration-whatsapp/schema.ts):
// copied, the package root is server-only. Keep it the same.
import {
  createSelectSchema,
  integrationWhatsappModel,
} from "@chatbotx.io/database/schema"
import { zodBigintAsString } from "@chatbotx.io/utils"

export const integrationWhatsappResource = createSelectSchema(
  integrationWhatsappModel,
  {
    id: zodBigintAsString(),
    inboxId: zodBigintAsString(),
  },
).pick({
  id: true,
  name: true,
  inboxId: true,
  displayPhoneNumber: true,
  tokenRefreshError: true,
  phoneNumberId: true,
  wabaId: true,
  hasCapiScope: true,
  capiScopeCheckedAt: true,
  datasetId: true,
})
