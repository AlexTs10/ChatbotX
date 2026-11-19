// apps/builder/src/features/facebook-lead-ad-automation/queries for the SPA:
// the Rust queries of crates/api/src/rpc_procedures/ads.rs (the public
// `facebookLeadAds.list` / `get` routes).
import { serverQuery } from "@/lib/server-action"
import type {
  ListFacebookLeadAdsRequest,
  ListFacebookLeadAdsResponse,
} from "../schema/query"
import type { FacebookLeadAdsAutomationResource } from "../schema/resource"

export const listFacebookLeadAdsAutomations = (
  input: ListFacebookLeadAdsRequest & { workspaceId: string },
) =>
  serverQuery<ListFacebookLeadAdsResponse>(
    "listFacebookLeadAdsAutomations",
    input.workspaceId,
    [input],
  )

export const getFacebookLeadAdAutomation = (workspaceId: string, id: string) =>
  serverQuery<FacebookLeadAdsAutomationResource>(
    "getFacebookLeadAdAutomation",
    workspaceId,
    [workspaceId, id],
  )
