// apps/builder/src/features/ads/queries/switcher.ts for the SPA: the Rust
// `getAdsSwitcherData` query (crates/api/src/rpc_procedures/ads.rs).
import type { WhatsappCredentialPublic } from "@chatbotx.io/database/partials"
import { serverQuery } from "@/lib/server-action"

export type AdsSwitcherIntegration = {
  id: string
  name: string
  displayPhoneNumber: string
  inboxId: string
  hasCapiScope: boolean
  isCoexist: boolean
}

export type AdsSwitcherChannelIntegration = {
  id: string
  name: string
}

export type AdsSwitcherData = {
  integrations: AdsSwitcherIntegration[]
  whatsappCredentialPublic: WhatsappCredentialPublic | null
  oauthCallbackUrl: string
  messengerIntegrations: AdsSwitcherChannelIntegration[]
  instagramIntegrations: AdsSwitcherChannelIntegration[]
  workspaceCreatedAt: Date
}

export const getAdsSwitcherData = (workspaceId: string) =>
  serverQuery<AdsSwitcherData>("getAdsSwitcherData", workspaceId, [
    workspaceId,
  ])
