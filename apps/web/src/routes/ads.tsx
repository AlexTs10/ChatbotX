// page: app/space/[workspaceId]/ads/page.tsx
// page: app/space/[workspaceId]/ads/analytics/page.tsx
// page: app/space/[workspaceId]/ads/connect-accounts/page.tsx
// page: app/space/[workspaceId]/ads/conversion-events/page.tsx
// page: app/space/[workspaceId]/dashboard/ads/page.tsx
// page: app/space/[workspaceId]/dashboard/ads/[channel]/page.tsx
// page: app/space/[workspaceId]/dashboard/ads/google/page.tsx
// page: app/space/[workspaceId]/dashboard/ads/conversion-events/page.tsx
// page: app/space/[workspaceId]/messaging-ads/page.tsx
// page: app/space/[workspaceId]/messaging-ads/[channel]/page.tsx
// page: app/space/[workspaceId]/fb-lead-ads/page.tsx
// page: app/space/[workspaceId]/fb-lead-ads/create/page.tsx
// page: app/space/[workspaceId]/fb-lead-ads/[id]/page.tsx
// page: app/space/[workspaceId]/fb-marketing-messages/page.tsx
// page: app/space/[workspaceId]/fb-marketing-messages/create/page.tsx
// page: app/space/[workspaceId]/fb-marketing-messages/[id]/page.tsx
// The ads screens (Q.D7). The builder server components' data comes from the
// Rust queries of crates/api/src/rpc_procedures/ads.rs (`serverQuery`); the
// super-admin guards (`resolveGuardedWorkspaceId(..., "superAdmin")`) show
// the 404 page, as `notFound()` does.
import {
  type AdsEligibleChannelType,
  adsEligibleChannelTypes,
  DEFAULT_ADS_CONVERSION_CHANNEL,
} from "@chatbotx.io/utils/channel"
import { useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { type ReactNode, Suspense, useMemo } from "react"
import { AppBreadcrumb } from "@/components/app-breadcrumb"
import { MessagingAdsToolView } from "@/features/ads-campaign/components/messaging-ads-tool-view"
import { selectMessagingAdsToolIntegration } from "@/features/ads-campaign/lib/select-tool-integration"
import { resolveMessagingAdsToolRedirectChannel } from "@/features/ads-campaign/lib/tool-channels"
import {
  buildMessagingAdsToolPath,
  MESSAGING_ADS_TOOL_INTEGRATION_PARAM,
} from "@/features/ads-campaign/lib/tool-path"
import {
  checkMessagingAdsConnectionState,
  listActiveMessagingAdsIntegrationIds,
  listMessagingAdsToolIntegrations,
} from "@/features/ads-campaign/queries/tool-integrations"
import { AdsAnalyticsView } from "@/features/ads/components/ads-analytics-view"
import { AdsReferralStats } from "@/features/ads/components/ads-referral-stats"
import { ConversionEventsView } from "@/features/ads/components/conversion-events-view"
import { resolveChannelIntegrations } from "@/features/ads/lib/resolve-channel-integrations"
import { resolveSelectedIntegration } from "@/features/ads/lib/select-account"
import { getConversionEventsData } from "@/features/ads/queries/conversion-rules"
import { getAdsSwitcherData } from "@/features/ads/queries/switcher"
import {
  adsAnalyticsSearchParamsCache,
  parseAnalyticsDateRange,
} from "@/features/ads/schema/analytics"
import { conversionEventsSearchParamsCache } from "@/features/ads/schema/conversion-events"
import { CreateFacebookLeadAdAutomationForm } from "@/features/facebook-lead-ad-automation/components/create-facebook-lead-ad-automation"
import { EditFacebookLeadAdAutomationForm } from "@/features/facebook-lead-ad-automation/components/edit-facebook-lead-ad-automation-form"
import { FacebookLeadAdsTable } from "@/features/facebook-lead-ad-automation/components/facebook-lead-ads-table"
import {
  getFacebookLeadAdAutomation,
  listFacebookLeadAdsAutomations,
} from "@/features/facebook-lead-ad-automation/queries"
import { listFacebookLeadAdsSearchParamsCache } from "@/features/facebook-lead-ad-automation/schema/query"
import { CampaignDetail } from "@/features/facebook-marketing-messages/components/campaign-detail"
import { CreateMarketingMessage } from "@/features/facebook-marketing-messages/components/create-marketing-message"
import { EditMarketingMessage } from "@/features/facebook-marketing-messages/components/edit-marketing-message"
import { GrantPanel } from "@/features/facebook-marketing-messages/components/grant-panel"
import { MarketingMessagesView } from "@/features/facebook-marketing-messages/components/marketing-messages-view"
import { parseMarketingMessagesTab } from "@/features/facebook-marketing-messages/lib/tab"
import {
  getMarketingMessagePage,
  getMarketingMessagesPage,
  resolveMarketingMessagesGrant,
} from "@/features/facebook-marketing-messages/queries"
import { GoogleAdsStatsView } from "@/features/integration-google-ads/components/stats/google-ads-stats-view"
import { googleAdsStatsSearchParamsCache } from "@/features/integration-google-ads/schema/stats-search-params"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import { serverQuery } from "@/lib/server-action"
import { useAdsNav } from "@/routes/dashboard"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"
import { Redirect } from "@/shims/redirect"

function useIds() {
  return useParams({ strict: false }) as {
    workspaceId: string
    id?: string
    channel?: string
  }
}

function useSearch() {
  const searchParams = useSearchParams()
  const key = searchParams.toString()
  return {
    key,
    params: Object.fromEntries(searchParams) as Record<string, string>,
    // `buildRedirectSearch`: the whole query string, kept as it is.
    redirect: key ? `?${key}` : "",
  }
}

/** One page load, keyed by name and inputs (`null` = failed / not found). */
function usePageQuery<T>(key: unknown[], load: () => Promise<T>) {
  const { workspaceId } = useIds()
  return useSuspenseQuery({
    queryKey: ["spa", ...key, workspaceId],
    queryFn: () => load().catch(() => null),
  }).data
}

/** `resolveGuardedWorkspaceId(..., "superAdmin")`: 404 for others. */
function SuperAdmin(props: { children: ReactNode }) {
  const { permissions } = useWorkspacePage()
  if (!hasWorkspacePermission(permissions, "superAdmin")) {
    return <NotFoundPage />
  }
  return <Suspense>{props.children}</Suspense>
}

function ToolsCrumbs(props: { items: { label: string; href: string }[] }) {
  const t = useTranslations()
  const { workspaceId } = useIds()
  return (
    <AppBreadcrumb
      items={[
        { label: t("tools.title"), href: `/space/${workspaceId}/tools` },
        ...props.items,
      ]}
    />
  )
}

// --- `ads/*`: old bookmarks, redirected to the dashboard ---------------------

export function AdsPage() {
  const { workspaceId } = useIds()
  return <Redirect href={`/space/${workspaceId}/dashboard/ads`} />
}

export function AdsAnalyticsRedirectPage() {
  const { workspaceId } = useIds()
  const { redirect } = useSearch()
  return <Redirect href={`/space/${workspaceId}/dashboard/ads${redirect}`} />
}

export function AdsConversionEventsRedirectPage() {
  const { workspaceId } = useIds()
  const { redirect } = useSearch()
  return (
    <Redirect
      href={`/space/${workspaceId}/dashboard/ads/conversion-events${redirect}`}
    />
  )
}

// --- `dashboard/ads/*` --------------------------------------------------------

/** `dashboard/ads/page.tsx`: the channel of `?channel=`, else the default. */
export function DashboardAdsRedirectPage() {
  const { workspaceId } = useIds()
  const { params, redirect } = useSearch()
  const parsed = adsEligibleChannelTypes.safeParse(params.channel)
  const channel = parsed.success ? parsed.data : DEFAULT_ADS_CONVERSION_CHANNEL
  return (
    <Redirect href={`/space/${workspaceId}/dashboard/ads/${channel}${redirect}`} />
  )
}

export function DashboardAdsChannelPage() {
  const parsed = adsEligibleChannelTypes.safeParse(useIds().channel)
  if (!parsed.success) {
    return <NotFoundPage />
  }
  return (
    <SuperAdmin>
      <AdsChannelAnalytics channel={parsed.data} />
    </SuperAdmin>
  )
}

function AdsChannelAnalytics({ channel }: { channel: AdsEligibleChannelType }) {
  const { workspaceId } = useIds()
  const { key, params } = useSearch()
  const nav = useAdsNav()
  const search = adsAnalyticsSearchParamsCache.parse(params)
  const range = { ...search, channel }
  const resolvedRange = parseAnalyticsDateRange(search)
  const switcherData = usePageQuery(["getAdsSwitcherData"], () =>
    getAdsSwitcherData(workspaceId),
  )
  const channelIntegrations = switcherData
    ? resolveChannelIntegrations(channel, switcherData)
    : []
  const requestedIntegrationId =
    search.channelAccount || (channel === "whatsapp" ? search.account : "")
  const selectedChannelIntegration =
    channelIntegrations.find(
      (integration) => integration.id === requestedIntegrationId,
    ) ?? null
  const selectedId = selectedChannelIntegration?.id
  // `perChannelIntegrationIds`: only the selected channel's column is set.
  const scope = {
    workspaceId,
    from: range.from,
    to: range.to,
    tz: range.tz,
    channel,
    integrationWhatsappId: channel === "whatsapp" ? selectedId : undefined,
    integrationMessengerId: channel === "messenger" ? selectedId : undefined,
    integrationInstagramId: channel === "instagram" ? selectedId : undefined,
    adAccountId: range.adAccount,
  }
  // biome-ignore lint/correctness/useExhaustiveDependencies: the search string and selection are the key
  const promises = useMemo(
    () =>
      serverQuery<never>("getAdsAnalytics", workspaceId, [workspaceId, scope]),
    [key, workspaceId, channel, selectedId],
  )
  if (!switcherData) {
    return <NotFoundPage />
  }

  return (
    <div className="flex gap-6">
      {nav}
      <div className="flex min-w-0 flex-1 flex-col gap-5">
        <Suspense>
          <AdsAnalyticsView
            channel={channel}
            channelIntegrations={channelIntegrations}
            promises={promises}
            range={range}
            selectedChannelIntegrationId={selectedId ?? null}
            workspaceCreatedAt={new Date(switcherData.workspaceCreatedAt)}
            workspaceId={workspaceId}
          />
        </Suspense>
        {selectedChannelIntegration ? null : (
          <AdsReferralStats
            channel={channel}
            range={{
              from: resolvedRange.from,
              to: resolvedRange.to,
              tz: resolvedRange.timezone,
            }}
            workspaceId={workspaceId}
          />
        )}
      </div>
    </div>
  )
}

export function DashboardGoogleAdsPage() {
  return (
    <SuperAdmin>
      <GoogleAdsStats />
    </SuperAdmin>
  )
}

type GoogleAdsStatsPage = Omit<
  Parameters<typeof GoogleAdsStatsView>[0],
  "action" | "channel" | "referenceNow" | "workspaceCreatedAt"
> & { workspaceCreatedAt: string }

function GoogleAdsStats() {
  const { workspaceId } = useIds()
  const { key, params } = useSearch()
  const nav = useAdsNav()
  const search = googleAdsStatsSearchParamsCache.parse(params)
  // One instant per page load, as the builder takes one per request.
  const referenceNow = useMemo(() => new Date().toISOString(), [])
  const page = usePageQuery(["getGoogleAdsStatsPage", key], () =>
    serverQuery<GoogleAdsStatsPage>("getGoogleAdsStatsPage", workspaceId, [
      workspaceId,
      search,
    ]),
  )
  if (!page) {
    return <NotFoundPage />
  }
  return (
    <div className="flex flex-col gap-6 md:flex-row">
      {nav}
      <div className="min-w-0 flex-1">
        <GoogleAdsStatsView
          action={search.action}
          channel={search.channel}
          connected={page.connected}
          googleReport={page.googleReport ?? undefined}
          referenceNow={referenceNow}
          stats={page.stats}
          syncedActions={page.syncedActions}
          workspaceCreatedAt={new Date(page.workspaceCreatedAt)}
        />
      </div>
    </div>
  )
}

export function DashboardConversionEventsPage() {
  return (
    <SuperAdmin>
      <ConversionEvents />
    </SuperAdmin>
  )
}

function ConversionEvents() {
  const { workspaceId } = useIds()
  const { params } = useSearch()
  const search = conversionEventsSearchParamsCache.parse(params)
  const data = usePageQuery(["getConversionEventsData"], () =>
    getConversionEventsData(workspaceId),
  )
  const switcherData = usePageQuery(["getAdsSwitcherData"], () =>
    getAdsSwitcherData(workspaceId),
  )
  const promises = useMemo(() => Promise.all([data as NonNullable<typeof data>]), [data])
  if (!(data && switcherData)) {
    return <NotFoundPage />
  }
  return (
    <Suspense>
      <ConversionEventsView
        oauthCallbackUrl={switcherData.oauthCallbackUrl}
        promises={promises}
        selectedAccount={resolveSelectedIntegration(
          data.whatsappIntegrations,
          search.account,
        )}
        switcherIntegrations={switcherData.integrations}
        whatsappCredentialPublic={switcherData.whatsappCredentialPublic}
        workspaceId={workspaceId}
      />
    </Suspense>
  )
}

// --- `messaging-ads/*` --------------------------------------------------------

export function MessagingAdsRedirectPage() {
  const { workspaceId } = useIds()
  const { params, redirect } = useSearch()
  const channel = resolveMessagingAdsToolRedirectChannel(params.channel)
  return (
    <Redirect href={buildMessagingAdsToolPath({ workspaceId, channel }) + redirect} />
  )
}

export function MessagingAdsChannelPage() {
  const parsed = adsEligibleChannelTypes.safeParse(useIds().channel)
  if (!parsed.success) {
    return <NotFoundPage />
  }
  return (
    <SuperAdmin>
      <MessagingAdsTool channel={parsed.data} />
    </SuperAdmin>
  )
}

function MessagingAdsTool({ channel }: { channel: AdsEligibleChannelType }) {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const { params } = useSearch()
  const requestedId = params[MESSAGING_ADS_TOOL_INTEGRATION_PARAM] ?? ""
  const page = usePageQuery(
    ["messagingAdsTool", channel, requestedId],
    async () => {
      const input = { workspaceId, channel }
      const [{ integrations, hasUnsupportedIntegrations }, activeIntegrationIds] =
        await Promise.all([
          listMessagingAdsToolIntegrations(input),
          listActiveMessagingAdsIntegrationIds(input),
        ])
      const selectedIntegration = selectMessagingAdsToolIntegration({
        integrations,
        requestedId,
        activeIntegrationIds,
      })
      const initialConnectionState = selectedIntegration
        ? await checkMessagingAdsConnectionState({
            ...input,
            integrationId: selectedIntegration.id,
          })
        : null
      return {
        integrations,
        hasUnsupportedIntegrations,
        selectedIntegration,
        initialConnectionState,
      }
    },
  )
  if (!page) {
    return <NotFoundPage />
  }
  return (
    <div className="flex flex-col gap-4">
      <ToolsCrumbs items={[{ label: t("clickToMessageAds.title"), href: "" }]} />
      <Suspense>
        <MessagingAdsToolView
          channel={channel}
          hasUnsupportedIntegrations={page.hasUnsupportedIntegrations}
          initialConnectionState={page.initialConnectionState}
          integrations={page.integrations}
          selectedIntegration={page.selectedIntegration}
          workspaceId={workspaceId}
        />
      </Suspense>
    </div>
  )
}

// --- `fb-lead-ads/*` ----------------------------------------------------------

function useLeadAdsCrumb() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  return {
    label: t("facebookLeadAdsAutomation.title"),
    href: `/space/${workspaceId}/fb-lead-ads`,
  }
}

export function FbLeadAdsPage() {
  const { workspaceId } = useIds()
  const { key, params } = useSearch()
  const crumb = useLeadAdsCrumb()
  // biome-ignore lint/correctness/useExhaustiveDependencies: the search string is the key
  const promises = useMemo(() => {
    const search = listFacebookLeadAdsSearchParamsCache.parse(params)
    return Promise.all([
      listFacebookLeadAdsAutomations({ ...search, workspaceId }),
    ])
  }, [key, workspaceId])
  return (
    <div className="flex flex-col gap-4">
      <ToolsCrumbs items={[{ ...crumb, href: "" }]} />
      <Suspense>
        <FacebookLeadAdsTable promises={promises} workspaceId={workspaceId} />
      </Suspense>
    </div>
  )
}

export function CreateFbLeadAdPage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const crumb = useLeadAdsCrumb()
  return (
    <div className="flex flex-col gap-4">
      <ToolsCrumbs
        items={[crumb, { label: t("facebookLeadAdsAutomation.create"), href: "" }]}
      />
      <CreateFacebookLeadAdAutomationForm workspaceId={workspaceId} />
    </div>
  )
}

export function EditFbLeadAdPage() {
  const { workspaceId, id = "" } = useIds()
  const crumb = useLeadAdsCrumb()
  const automation = usePageQuery(["getFacebookLeadAdAutomation", id], () =>
    getFacebookLeadAdAutomation(workspaceId, id),
  )
  if (!automation) {
    return <NotFoundPage />
  }
  return (
    <div className="flex flex-col gap-4">
      <ToolsCrumbs items={[crumb, { label: automation.name, href: "" }]} />
      <EditFacebookLeadAdAutomationForm
        initialData={{
          id: automation.id,
          name: automation.name,
          pageName: automation.pageName,
          formId: automation.formId,
          formName: automation.formName,
          fieldMapping: automation.fieldMapping,
        }}
        workspaceId={workspaceId}
      />
    </div>
  )
}

// --- `fb-marketing-messages/*` ------------------------------------------------

function useMarketingCrumb() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  return {
    label: t("facebookMarketingMessages.title"),
    href: `/space/${workspaceId}/fb-marketing-messages`,
  }
}

export function FbMarketingMessagesPage() {
  const { workspaceId } = useIds()
  const { params } = useSearch()
  const crumb = useMarketingCrumb()
  const page = usePageQuery(["getMarketingMessagesPage"], () =>
    getMarketingMessagesPage(workspaceId),
  )
  if (!page) {
    return <NotFoundPage />
  }
  return (
    <div className="flex flex-col gap-4">
      <ToolsCrumbs items={[{ ...crumb, href: "" }]} />
      <Suspense>
        <MarketingMessagesView
          counts={page.counts}
          grantState={page.grantState}
          rows={page.rows}
          tab={parseMarketingMessagesTab(params.tab)}
          workspaceId={workspaceId}
        />
      </Suspense>
    </div>
  )
}

export function CreateFbMarketingMessagePage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const crumb = useMarketingCrumb()
  const grant = usePageQuery(["resolveMarketingMessagesGrant"], () =>
    resolveMarketingMessagesGrant(workspaceId),
  )
  if (!grant) {
    return <NotFoundPage />
  }
  if (grant.state !== "valid") {
    return <Redirect href={crumb.href} />
  }
  return (
    <div className="flex flex-col gap-4">
      <ToolsCrumbs items={[crumb, { label: t("actions.create"), href: "" }]} />
      <Suspense>
        <CreateMarketingMessage workspaceId={workspaceId} />
      </Suspense>
    </div>
  )
}

export function EditFbMarketingMessagePage() {
  const { workspaceId, id = "" } = useIds()
  const crumb = useMarketingCrumb()
  const page = usePageQuery(["getMarketingMessagePage", id], () =>
    getMarketingMessagePage(workspaceId, id),
  )
  if (!page) {
    return <NotFoundPage />
  }
  const { grantState, row, stats } = page
  return (
    <div className="flex flex-col gap-4">
      <ToolsCrumbs items={[crumb, { label: row.name, href: "" }]} />
      <CampaignDetail campaign={row} stats={stats} workspaceId={workspaceId} />
      {grantState === "valid" ? (
        <Suspense>
          <EditMarketingMessage row={row} workspaceId={workspaceId} />
        </Suspense>
      ) : (
        <GrantPanel state={grantState} workspaceId={workspaceId} />
      )}
    </div>
  )
}
