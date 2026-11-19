// apps/builder/src/features/ads/queries/conversion-rules.ts for the SPA: the
// Rust `getConversionEventsData` query (crates/api/src/rpc_procedures/ads.rs).
import type { AdsConversionRuleResource } from "@chatbotx.io/business/ads-conversion/schema"
import type { AutomatedResponseResource } from "@/features/automated-response/schema/resource"
import type { TagResource } from "@/features/tags/schema/resource"
import { serverQuery } from "@/lib/server-action"

type IdName = { id: string; name: string }
type Template = { id: string; name: string; language: string; status: string }

export type ConversionEventsWhatsappIntegration = IdName
export type ConversionEventsMessengerIntegration = IdName
export type ConversionEventsInstagramIntegration = IdName

export type ConversionEventsData = {
  whatsappIntegrations: ConversionEventsWhatsappIntegration[]
  whatsappTemplates: (Template & { integrationWhatsappId: string })[]
  messengerIntegrations: ConversionEventsMessengerIntegration[]
  messengerTemplates: (Template & { integrationMessengerId: string })[]
  instagramIntegrations: ConversionEventsInstagramIntegration[]
  rules: AdsConversionRuleResource[]
  tags: TagResource[]
  automatedResponses: AutomatedResponseResource[]
}

export const getConversionEventsData = (workspaceId: string) =>
  serverQuery<ConversionEventsData>("getConversionEventsData", workspaceId, [
    workspaceId,
  ])
