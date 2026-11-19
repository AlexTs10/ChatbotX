// page: app/space/[workspaceId]/(settings)/settings/layout.tsx
// page: app/space/[workspaceId]/(settings)/settings/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/general/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/admins/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/channels/layout.tsx
// page: app/space/[workspaceId]/(settings)/settings/channels/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/channels/api/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/channels/telegram/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/channels/webchat/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/layout.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/openai/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/claude/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/deepseek/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/gemini/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/muse/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/openrouter/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/openai-compatible/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/google-sheets/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/inbox-teams/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/user-persistent-menus/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/user-persistent-menus/create/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/user-persistent-menus/[menuId]/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/inbox-teams/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/channels/smtp/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/active-campaign/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/drip/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/get-response/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/klaviyo/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/mailchimp/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/mailer-lite/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/moosend/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/sendgrid/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/make/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/chatbotx-mcp/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/paypal/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/workspace-token/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/channels/tiktok/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/channels/zalo/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/channels/threads/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/google-ads/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/facebook-ads/page.tsx
// page: app/space/[workspaceId]/(settings)/settings/integrations/decision-connections/page.tsx
// `resolveGuardedWorkspaceId(params, "superAdmin")`: 404 without superAdmin.
// The channel policy (creatable / connected channels) is the page loader
// `pages.settingsChannels`; the other data comes from the builder queries.
import {
  type ChannelType,
  CREATABLE_CHANNELS,
  MANAGEABLE_CHANNELS,
} from "@chatbotx.io/database/partials"
import { buttonVariants } from "@chatbotx.io/ui/components/ui/button"
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@chatbotx.io/ui/components/ui/card"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet, useParams } from "@tanstack/react-router"
import Link from "next/link"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { type ReactNode, Suspense, useMemo } from "react"
import { ChannelsAccordionShell } from "@/app/space/[workspaceId]/(settings)/settings/channels/channels-accordion-shell"
import { IntegrationsAccordionShell } from "@/app/space/[workspaceId]/(settings)/settings/integrations/integrations-accordion-shell"
import { SettingsTab } from "@/app/space/[workspaceId]/(settings)/settings/tab"
import { isCommunity } from "@/env"
import { ListInboxTeams } from "@/enterprise/features/inbox-teams/list-inbox-teams"
import { listInboxTeams } from "@/enterprise/features/inbox-teams/queries"
import { ApiManage } from "@/features/integration-api/api-manage"
import { listIntegrationApis } from "@/features/integration-api/queries"
import { ClaudeConnect } from "@/features/integration-claude/claude-connect"
import { findIntegrationClaude } from "@/features/integration-claude/queries"
import { DeepSeekConnect } from "@/features/integration-deepseek/deepseek-connect"
import { findIntegrationDeepSeek } from "@/features/integration-deepseek/queries"
import { GeminiAIManage } from "@/features/integration-gemini/gemini-manage"
import { findIntegrationGemini } from "@/features/integration-gemini/queries"
import { MuseConnect } from "@/features/integration-muse/muse-connect"
import { findIntegrationMuse } from "@/features/integration-muse/queries"
import { OpenAIConnect } from "@/features/integration-openai/openai-connect"
import { findIntegrationOpenAI } from "@/features/integration-openai/queries"
import { OpenaiCompatibleConnect } from "@/features/integration-openai-compatible/openai-compatible-connect"
import { listIntegrationOpenaiCompatible } from "@/features/integration-openai-compatible/queries"
import { OpenRouterConnect } from "@/features/integration-openrouter/openrouter-connect"
import { findIntegrationOpenRouter } from "@/features/integration-openrouter/queries"
import { listIntegrationTelegrams } from "@/features/integration-telegram/queries"
import { listIntegrationThreads } from "@/features/integration-threads/queries"
import { ThreadsManage } from "@/features/integration-threads/threads-manage"
import { listIntegrationTiktoks } from "@/features/integration-tiktok/queries"
import { TiktokManage } from "@/features/integration-tiktok/tiktok-manage"
import { listIntegrationZalo } from "@/features/integration-zalo/queries"
import { ZaloManage } from "@/features/integration-zalo/zalo-manage"
import { TelegramManage } from "@/features/integration-telegram/telegram-manage"
import { listIntegrationWebchats } from "@/features/integration-webchat/queries"
import { listIntegrationWebchatsRequest } from "@/features/integration-webchat/schema/query"
import { WebchatTable } from "@/features/integration-webchat/webchat-table"
import { listWorkspaceMembers } from "@/features/workspace-members/queries"
import { getWorkspaceMembersSearchParamsCache } from "@/features/workspace-members/schema/query"
import { WorkspaceMembersTable } from "@/features/workspace-members/workspace-members-table"
import { ManageWorkspaceTokens } from "@/features/workspaces/manage-workspace-tokens"
import { listWorkspaceTokens } from "@/features/workspaces/queries/workspace-tokens.query"
import { UpdateWorkspaceForm } from "@/features/workspaces/update-workspace-form"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import {
  HolidaysCard,
  HolidaysCardFallback,
} from "@/features/working-hours/components/holidays-card"
import {
  ListWorkingHours,
  ListWorkingHoursFallback,
} from "@/features/working-hours/components/list-working-hours"
import { getWorkingHoursListPage } from "@/features/working-hours/queries"
import { GoogleSheetsManage } from "@/features/integration-google-sheets/google-sheets-manage"
import type { IntegrationGoogleSheetsResource } from "@/features/integration-google-sheets/schema"
import {
  ResponseSlaCard,
  ResponseSlaCardFallback,
} from "@/features/response-sla/components/response-sla-card"
import type { WorkingHoursResource } from "@/features/working-hours/schema/resource"
import { useQueryPromise } from "@/lib/query/use-query-promise"
import { UserPersistentMenuForm } from "@/features/user-persistent-menus/components/user-persistent-menu-form"
import { UserPersistentMenuList } from "@/features/user-persistent-menus/components/user-persistent-menu-list"
import {
  findUserPersistentMenu,
  listUserPersistentMenus,
} from "@/features/user-persistent-menus/queries"
import { ManageActiveCampaign } from "@/features/integration-active-campaign/components/manage-active-campaign"
import { ManageDrip } from "@/features/integration-drip/components/manage-drip"
import { ManageGetResponse } from "@/features/integration-get-response/components/manage-get-response"
import { ManageKlaviyo } from "@/features/integration-klaviyo/components/manage-klaviyo"
import { ManageMailchimp } from "@/features/integration-mailchimp/components/manage-mailchimp"
import { ManageMailerLite } from "@/features/integration-mailer-lite/components/manage-mailer-lite"
import { ManageMake } from "@/features/integration-make/components/manage-make"
import { ManageMoosend } from "@/features/integration-moosend/components/manage-moosend"
import { ManageSendGrid } from "@/features/integration-sendgrid/components/manage-sendgrid"
import { listIntegrationSmtps } from "@/features/integration-smtp/queries"
import { SmtpManage } from "@/features/integration-smtp/smtp-manage"
import type { DecisionConnectionSafe } from "@chatbotx.io/business"
import { DecisionConnectionsSettings } from "@/features/decision-connections/components/decision-connections-settings"
import { AdAccountsSection } from "@/features/integration-facebook-ads/components/ad-accounts-section"
import { FacebookAdsManage } from "@/features/integration-facebook-ads/facebook-ads-manage"
import { needsFacebookAdsReconnect } from "@/features/integration-facebook-ads/lib/needs-reconnect"
import type { IntegrationFacebookAdsResource } from "@/features/integration-facebook-ads/schema"
import { GoogleAdsSettings } from "@/features/integration-google-ads/components/google-ads-settings"
import { SESSION_QUERY_PARAM } from "@/features/integration-google-ads/lib/constants"
import { parseConnectErrorParam } from "@/features/integration-google-ads/lib/load-settings-session"
import type { GoogleAdsSettingsView } from "@/features/integration-google-ads/lib/to-settings-view"
import type { GoogleAdsConsentView } from "@/features/integration-google-ads/schema/integration"
import { ManageChatbotxMcp } from "@/features/mcp-oauth/manage-chatbotx-mcp"
import type { McpOAuthGrantDto } from "@/features/mcp-oauth/schema/grant-dto"
import { ManagePaypal } from "@/features/payment-providers/components/manage-paypal"
import { ManageStripe } from "@/features/payment-providers/components/manage-stripe"
import type { PaymentProviderResource } from "@/features/payment-providers/schema/public"
import { loadPage, serverQuery } from "@/lib/server-action"
import { workspaceSettingsGeneralPath } from "@/lib/workspace/settings-paths"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"
import { Redirect } from "@/shims/redirect"

function useWorkspaceId() {
  return (useParams({ strict: false }) as { workspaceId: string }).workspaceId
}

export function SettingsLayout() {
  const { permissions, scheduledForDeletion } = useWorkspacePage()
  if (!hasWorkspacePermission(permissions, "superAdmin")) {
    return <NotFoundPage />
  }

  return (
    <>
      <SettingsTab scheduledForDeletion={scheduledForDeletion} />
      <div>
        <Outlet />
      </div>
    </>
  )
}

// Absolute on purpose (a relative target resolves against the current URL).
export function SettingsIndexPage() {
  return <Redirect href={workspaceSettingsGeneralPath(useWorkspaceId())} />
}

export function SettingsGeneralPage() {
  const workspaceId = useWorkspaceId()
  const { allWorkspaces, permissions, isSupportSession, consentRequired } =
    useWorkspacePage()
  const workspace = allWorkspaces.find((item) => item.id === workspaceId)
  if (!workspace) {
    return <NotFoundPage />
  }

  return (
    <UpdateWorkspaceForm
      canManageSupportAccess={
        hasWorkspacePermission(permissions, "superAdmin") && !isSupportSession
      }
      consentRequired={consentRequired}
      workspace={workspace}
    />
  )
}

// `settings/inbox-teams/page.tsx`: the teams list; working hours, holidays
// and the response SLA have no Rust domain yet.
function settled<T>(value: T): Promise<T> {
  return Object.assign(Promise.resolve(value), {
    status: "fulfilled",
    value,
  })
}

export function SettingsInboxTeamsPage() {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(["inboxTeams", workspaceId], () =>
    Promise.all([
      listInboxTeams({ workspaceId }),
      listWorkspaceMembers({
        workspaceId,
        ...getWorkspaceMembersSearchParamsCache.parse({}),
      }),
    ] as const),
  )
  const { data: workingHours } = useSuspenseQuery({
    queryKey: ["spa", "workingHoursList", workspaceId],
    queryFn: () => getWorkingHoursListPage({ workspaceId }),
  })
  // Settled thenables: `use()` reads them without suspending again.
  const workingHoursPromises = useMemo(
    () =>
      settled<[WorkingHoursResource[], { value: string; label: string }[]]>([
        workingHours.workingHours,
        workingHours.members,
      ]),
    [workingHours],
  )
  const holidaysPromise = useMemo(
    () => settled(workingHours.holidays),
    [workingHours],
  )
  const responseSlaPromise = useMemo(
    () => settled(workingHours.responseSla),
    [workingHours],
  )
  return (
    <div className="flex flex-col gap-6">
      <Suspense>
        <ListInboxTeams promises={promises} workspaceId={workspaceId} />
      </Suspense>
      <Suspense fallback={<ListWorkingHoursFallback />}>
        <ListWorkingHours
          promises={workingHoursPromises}
          workspaceId={workspaceId}
        />
      </Suspense>
      <Suspense fallback={<HolidaysCardFallback />}>
        <HolidaysCard
          promise={holidaysPromise}
          workspaceId={workspaceId}
        />
      </Suspense>
      <Suspense fallback={<ResponseSlaCardFallback />}>
        <ResponseSlaCard promise={responseSlaPromise} workspaceId={workspaceId} />
      </Suspense>
    </div>
  )
}

export function SettingsUserPersistentMenusPage() {
  const workspaceId = useWorkspaceId()
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "listUserPersistentMenus", workspaceId],
    queryFn: () => listUserPersistentMenus({ workspaceId }),
  })
  return <UserPersistentMenuList menus={data.data} workspaceId={workspaceId} />
}

function UserPersistentMenuCard(props: {
  title: "actions.createFeature" | "actions.editFeature"
  children: ReactNode
}) {
  const t = useTranslations()
  return (
    <Card>
      <CardHeader>
        <CardTitle className="font-bold text-xl">
          {t(props.title, { feature: t("fields.userPersistentMenu.label") })}
        </CardTitle>
      </CardHeader>
      <CardContent>{props.children}</CardContent>
    </Card>
  )
}

export function SettingsUserPersistentMenuCreatePage() {
  return (
    <UserPersistentMenuCard title="actions.createFeature">
      <UserPersistentMenuForm workspaceId={useWorkspaceId()} />
    </UserPersistentMenuCard>
  )
}

export function SettingsUserPersistentMenuEditPage() {
  const workspaceId = useWorkspaceId()
  const { menuId } = useParams({ strict: false }) as { menuId: string }
  const { data: menu } = useSuspenseQuery({
    queryKey: ["spa", "findUserPersistentMenu", workspaceId, menuId],
    queryFn: () => findUserPersistentMenu({ id: menuId, workspaceId }),
  })
  if (!menu) {
    return <NotFoundPage />
  }
  return (
    <UserPersistentMenuCard title="actions.editFeature">
      <UserPersistentMenuForm menu={menu} workspaceId={workspaceId} />
    </UserPersistentMenuCard>
  )
}

export function SettingsAdminsPage() {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(["workspaceMembers", workspaceId], () =>
    Promise.all([
      listWorkspaceMembers({
        workspaceId,
        ...getWorkspaceMembersSearchParamsCache.parse({}),
      }),
    ] as const),
  )

  return (
    <Card>
      <CardHeader>
        <CardTitle className="font-bold text-xl">{t("admins.title")}</CardTitle>
      </CardHeader>
      <CardContent>
        {/* ponytail: no quota in Rust yet (`pages.workspace` quota is empty), so never at the limit. */}
        <WorkspaceMembersTable promises={promises} teamMembersAtLimit={false} />
        {!isCommunity() && (
          <div className="mt-6 flex justify-center">
            <Link
              className={buttonVariants({ size: "sm", variant: "outline" })}
              href={`/space/${workspaceId}/audit-logs`}
            >
              {t("admins.adminActivityLogs")}
            </Link>
          </div>
        )}
      </CardContent>
    </Card>
  )
}

// ---- channels -------------------------------------------------------------

type ChannelPolicy = { creatable: ChannelType[]; connected: ChannelType[] }

/** `resolveChannelPolicy`: the visible rows and the creatable channels. */
function useChannelPolicy() {
  const workspaceId = useWorkspaceId()
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "settingsChannels", workspaceId],
    queryFn: () => loadPage<ChannelPolicy>("settingsChannels", { workspaceId }),
  })
  const visibleChannels = MANAGEABLE_CHANNELS.filter(
    (channel) =>
      !CREATABLE_CHANNELS.includes(channel) ||
      data.creatable.includes(channel) ||
      data.connected.includes(channel),
  )
  const canCreate = (channel: ChannelType) =>
    !CREATABLE_CHANNELS.includes(channel) || data.creatable.includes(channel)
  return { visibleChannels, canCreate }
}

export function SettingsChannelsLayout() {
  const { visibleChannels } = useChannelPolicy()

  return (
    <ChannelsAccordionShell visibleChannels={visibleChannels}>
      <Outlet />
    </ChannelsAccordionShell>
  )
}

// `?channel=<x>`: the legacy deep link goes to `settings/channels/<x>`,
// keeping the other params (the connect-error toast reads `error`).
export function SettingsChannelsIndexPage() {
  const workspaceId = useWorkspaceId()
  const searchParams = useSearchParams()
  const { visibleChannels } = useChannelPolicy()
  const channel = searchParams.get("channel") as ChannelType | null
  if (!(channel && visibleChannels.includes(channel))) {
    return null
  }
  const rest = new URLSearchParams(searchParams)
  rest.delete("channel")
  const qs = rest.size > 0 ? `?${rest.toString()}` : ""
  return (
    <Redirect
      href={`/space/${workspaceId}/settings/channels/${channel}${qs}`}
    />
  )
}

export function SettingsChannelPage() {
  const { channel } = useParams({ strict: false }) as { channel: ChannelType }
  const { visibleChannels, canCreate } = useChannelPolicy()
  // `requireVisibleChannel`.
  if (!visibleChannels.includes(channel)) {
    return <NotFoundPage />
  }
  switch (channel) {
    case "api":
      return <ApiChannel canCreate={canCreate(channel)} />
    case "telegram":
      return <TelegramChannel canCreate={canCreate(channel)} />
    case "webchat":
      return <WebchatChannel canCreate={canCreate(channel)} />
    case "smtp":
      return <SmtpChannel />
    case "tiktok":
      return <TiktokChannel canCreate={canCreate(channel)} />
    case "zalo":
      return <ZaloChannel canCreate={canCreate(channel)} />
    case "threads":
      return <ThreadsChannel canCreate={canCreate(channel)} />
    default:
      // ponytail: the other channel pages wait for their integration units.
      return null
  }
}

function ApiChannel({ canCreate }: { canCreate: boolean }) {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(["integrationApis", workspaceId], () =>
    listIntegrationApis({ workspaceId }),
  )
  return (
    <ApiManage
      canCreate={canCreate}
      promises={promises}
      workspaceId={workspaceId}
    />
  )
}

function TelegramChannel({ canCreate }: { canCreate: boolean }) {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(["integrationTelegrams", workspaceId], () =>
    Promise.all([
      listIntegrationTelegrams({ where: { workspaceId } }),
    ] as const),
  )
  return (
    <TelegramManage
      canCreate={canCreate}
      promises={promises}
      workspaceId={workspaceId}
    />
  )
}

/** The platform credential's public app id (`publicConfig.clientId`). */
function useChannelClientId(type: "tiktok" | "zalo" | "threads") {
  const workspaceId = useWorkspaceId()
  const { data } = useSuspenseQuery({
    queryKey: ["channelCredentialPage", workspaceId, type],
    queryFn: () =>
      serverQuery<{ clientId: string | null }>(
        "channelCredentialPage",
        workspaceId,
        [type],
      ),
  })
  return data.clientId
}

function TiktokChannel({ canCreate }: { canCreate: boolean }) {
  const workspaceId = useWorkspaceId()
  const clientId = useChannelClientId("tiktok")
  const promises = useQueryPromise(["integrationTiktoks", workspaceId], () =>
    Promise.all([listIntegrationTiktoks({ workspaceId })]),
  )
  return (
    <TiktokManage
      canCreate={canCreate}
      isEnabled={Boolean(clientId)}
      promises={promises}
      workspaceId={workspaceId}
    />
  )
}

function ZaloChannel({ canCreate }: { canCreate: boolean }) {
  const workspaceId = useWorkspaceId()
  const clientId = useChannelClientId("zalo")
  const promises = useQueryPromise(["integrationZalo", workspaceId], () =>
    Promise.all([listIntegrationZalo({ workspaceId })]),
  )
  return (
    <ZaloManage
      canCreate={canCreate}
      isEnabled={Boolean(clientId)}
      promises={promises}
      workspaceId={workspaceId}
    />
  )
}

function ThreadsChannel({ canCreate }: { canCreate: boolean }) {
  const workspaceId = useWorkspaceId()
  const clientId = useChannelClientId("threads")
  const promises = useQueryPromise(["integrationThreads", workspaceId], () =>
    Promise.all([listIntegrationThreads({ workspaceId })]),
  )
  return (
    <ThreadsManage
      canCreate={canCreate}
      promises={promises}
      publicConfig={clientId ? { clientId } : null}
      workspaceId={workspaceId}
    />
  )
}

function SmtpChannel() {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(["integrationSmtps", workspaceId], () =>
    listIntegrationSmtps({ workspaceId }),
  )
  return <SmtpManage promises={promises} workspaceId={workspaceId} />
}

function WebchatChannel({ canCreate }: { canCreate: boolean }) {
  const workspaceId = useWorkspaceId()
  const searchParams = useSearchParams()
  const promises = useQueryPromise(
    ["integrationWebchats", workspaceId, searchParams.toString()],
    () =>
      Promise.all([
        listIntegrationWebchats({
          ...listIntegrationWebchatsRequest.parse(
            Object.fromEntries(searchParams),
          ),
          workspaceId,
        }),
      ] as const),
  )
  return (
    <Suspense>
      <WebchatTable canCreate={canCreate} promises={promises} />
    </Suspense>
  )
}

// ---- integrations ---------------------------------------------------------

export function SettingsIntegrationsLayout() {
  return (
    <IntegrationsAccordionShell>
      <Outlet />
    </IntegrationsAccordionShell>
  )
}

// Index route: every accordion row collapsed.
export const SettingsIntegrationsIndexPage = () => null

export function SettingsIntegrationPage() {
  const { slug } = useParams({ strict: false }) as { slug: string }
  switch (slug) {
    case "openai":
      return <OpenAIIntegration />
    case "claude":
      return <ClaudeIntegration />
    case "deepseek":
      return <DeepSeekIntegration />
    case "gemini":
      return <GeminiIntegration />
    case "muse":
      return <MuseIntegration />
    case "openrouter":
      return <OpenRouterIntegration />
    case "openai-compatible":
      return <OpenaiCompatibleIntegration />
    case "workspace-token":
      return <WorkspaceTokens />
    case "stripe":
      return <StripeIntegration />
    case "google-sheets":
      return <GoogleSheetsIntegration />
    case "paypal":
      return <PaypalIntegration />
    case "make":
      return <MakeIntegration />
    case "chatbotx-mcp":
      return <ChatbotxMcpIntegration />
    case "google-ads":
      return <GoogleAdsIntegration />
    case "facebook-ads":
      return <FacebookAdsIntegration />
    case "decision-connections":
      return <DecisionConnectionsIntegration />
    case "active-campaign":
    case "drip":
    case "get-response":
    case "klaviyo":
    case "mailchimp":
    case "mailer-lite":
    case "moosend":
    case "sendgrid":
      return <EmailIntegration slug={slug} />
    default:
      // ponytail: the other integrations wait for their domain units.
      return null
  }
}

function GoogleSheetsIntegration() {
  const workspaceId = useWorkspaceId()
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "findIntegrationGoogleSheet", workspaceId],
    queryFn: () =>
      serverQuery<IntegrationGoogleSheetsResource | null>(
        "findIntegrationGoogleSheet",
        workspaceId,
        [],
      ),
  })
  return (
    <GoogleSheetsManage
      integrationGoogleSheets={data ?? undefined}
      workspaceId={workspaceId}
    />
  )
}

// page: app/space/[workspaceId]/(settings)/settings/integrations/stripe/page.tsx
function StripeIntegration() {
  const { workspaceId, data } = usePaymentProviders()
  return (
    <ManageStripe
      available={data.stripeAvailable}
      connection={data.data.find((row) => row.provider === "stripe") ?? null}
      workspaceId={workspaceId}
    />
  )
}

function usePaymentProviders() {
  const workspaceId = useWorkspaceId()
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "paymentProvidersPage", workspaceId],
    queryFn: () =>
      serverQuery<{
        data: PaymentProviderResource[]
        stripeAvailable: boolean
      }>("paymentProvidersPage", workspaceId, [workspaceId]),
  })
  return { workspaceId, data }
}

function PaypalIntegration() {
  const { workspaceId, data } = usePaymentProviders()
  return (
    <ManagePaypal
      connection={data.data.find((row) => row.provider === "paypal") ?? null}
      workspaceId={workspaceId}
    />
  )
}

function DecisionConnectionsIntegration() {
  const workspaceId = useWorkspaceId()
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "decisionConnectionsPage", workspaceId],
    queryFn: () =>
      serverQuery<{ connections: DecisionConnectionSafe[] }>(
        "decisionConnectionsPage",
        workspaceId,
        [],
      ),
  })
  return <DecisionConnectionsSettings connections={data.connections} />
}

function FacebookAdsIntegration() {
  const workspaceId = useWorkspaceId()
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "facebookAdsSettingsPage", workspaceId],
    queryFn: () =>
      serverQuery<{ integration: IntegrationFacebookAdsResource | null }>(
        "facebookAdsSettingsPage",
        workspaceId,
        [],
      ),
  })
  const integration = data.integration ?? undefined
  return (
    <div className="flex flex-col gap-4">
      <FacebookAdsManage
        integrationFacebookAds={integration}
        workspaceId={workspaceId}
      />
      {/* Connected Meta ad accounts are listed here (moved from the
          Analytics > Ads screen). */}
      {integration && (
        <AdAccountsSection
          facebookAds={{
            connected: true,
            needsReconnect: needsFacebookAdsReconnect(integration),
          }}
          workspaceId={workspaceId}
        />
      )}
    </div>
  )
}

function GoogleAdsIntegration() {
  const workspaceId = useWorkspaceId()
  const searchParams = useSearchParams()
  const sessionParam = searchParams.get(SESSION_QUERY_PARAM)
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "googleAdsSettingsPage", workspaceId, sessionParam],
    queryFn: () =>
      serverQuery<{
        setup:
          | (Omit<GoogleAdsSettingsView, "conversionActionsSyncedAt"> & {
              conversionActionsSyncedAt: string | null
            })
          | null
        isConfigured: boolean
        session: { id: string; status: string } | null
        consent: GoogleAdsConsentView
      }>("googleAdsSettingsPage", workspaceId, [sessionParam]),
  })
  const setup = data.setup && {
    ...data.setup,
    conversionActionsSyncedAt: data.setup.conversionActionsSyncedAt
      ? new Date(data.setup.conversionActionsSyncedAt)
      : null,
  }
  return (
    <GoogleAdsSettings
      connectError={parseConnectErrorParam(
        searchParams.get("connect_error") ?? undefined,
      )}
      consent={data.consent}
      initialSession={data.session}
      isConfigured={data.isConfigured}
      setup={setup}
      workspaceId={workspaceId}
    />
  )
}

function MakeIntegration() {
  const workspaceId = useWorkspaceId()
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "makeIntegrationPage", workspaceId],
    queryFn: () =>
      serverQuery<{ inviteUrl: string | null }>(
        "makeIntegrationPage",
        workspaceId,
        [],
      ),
  })
  return <ManageMake inviteUrl={data.inviteUrl ?? undefined} />
}

function ChatbotxMcpIntegration() {
  const workspaceId = useWorkspaceId()
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "chatbotxMcpPage", workspaceId],
    queryFn: () =>
      serverQuery<{
        grants: McpOAuthGrantDto[]
        mcpUrl: string
        builderUrl: string
      }>("chatbotxMcpPage", workspaceId, []),
  })
  return <ManageChatbotxMcp {...data} workspaceId={workspaceId} />
}

// The email-marketing rows: `integration<Name>Service.findByWorkspaceId`.
const EMAIL_INTEGRATIONS = {
  "active-campaign": ["activeCampaign", ManageActiveCampaign],
  drip: ["drip", ManageDrip],
  "get-response": ["getResponse", ManageGetResponse],
  klaviyo: ["klaviyo", ManageKlaviyo],
  mailchimp: ["mailchimp", ManageMailchimp],
  "mailer-lite": ["mailerLite", ManageMailerLite],
  moosend: ["moosend", ManageMoosend],
  sendgrid: ["sendGrid", ManageSendGrid],
} as const

function EmailIntegration({
  slug,
}: {
  slug: keyof typeof EMAIL_INTEGRATIONS
}) {
  const workspaceId = useWorkspaceId()
  const [provider, Manage] = EMAIL_INTEGRATIONS[slug]
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "emailIntegrationPage", workspaceId, provider],
    queryFn: () =>
      serverQuery<{ connected: boolean }>("emailIntegrationPage", workspaceId, [
        provider,
      ]),
  })
  return <Manage isConnected={data.connected} workspaceId={workspaceId} />
}

function WorkspaceTokens() {
  const workspaceId = useWorkspaceId()
  const { data: tokens } = useSuspenseQuery({
    queryKey: ["spa", "listWorkspaceTokens", workspaceId],
    queryFn: () => listWorkspaceTokens(workspaceId),
  })
  return <ManageWorkspaceTokens tokens={tokens} workspaceId={workspaceId} />
}

function OpenAIIntegration() {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(["integration", "openai", workspaceId], () =>
    Promise.all([findIntegrationOpenAI({ workspaceId })] as const),
  )
  return <OpenAIConnect promises={promises} workspaceId={workspaceId} />
}

function ClaudeIntegration() {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(["integration", "claude", workspaceId], () =>
    Promise.all([findIntegrationClaude({ workspaceId })] as const),
  )
  return <ClaudeConnect promises={promises} />
}

function DeepSeekIntegration() {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(
    ["integration", "deepseek", workspaceId],
    () => Promise.all([findIntegrationDeepSeek({ workspaceId })] as const),
  )
  return <DeepSeekConnect promises={promises} />
}

function GeminiIntegration() {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(["integration", "gemini", workspaceId], () =>
    Promise.all([findIntegrationGemini({ workspaceId })] as const),
  )
  return <GeminiAIManage promises={promises} />
}

function MuseIntegration() {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(["integration", "muse", workspaceId], () =>
    Promise.all([findIntegrationMuse({ workspaceId })] as const),
  )
  return <MuseConnect promises={promises} />
}

function OpenRouterIntegration() {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(
    ["integration", "openrouter", workspaceId],
    () => Promise.all([findIntegrationOpenRouter({ workspaceId })] as const),
  )
  return <OpenRouterConnect promises={promises} />
}

function OpenaiCompatibleIntegration() {
  const workspaceId = useWorkspaceId()
  const promises = useQueryPromise(
    ["integration", "openaiCompatible", workspaceId],
    () =>
      Promise.all([listIntegrationOpenaiCompatible({ workspaceId })] as const),
  )
  return <OpenaiCompatibleConnect promises={promises} />
}
