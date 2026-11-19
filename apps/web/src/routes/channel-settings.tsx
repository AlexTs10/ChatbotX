// page: app/space/[workspaceId]/(integrations)/whatsapps/[id]/layout.tsx
// page: app/space/[workspaceId]/(integrations)/whatsapps/[id]/settings/page.tsx
// page: app/space/[workspaceId]/(integrations)/whatsapps/[id]/account-healths/page.tsx
// page: app/space/[workspaceId]/(integrations)/whatsapps/[id]/flows/page.tsx
// page: app/space/[workspaceId]/(integrations)/whatsapps/[id]/message-templates/page.tsx
// page: app/space/[workspaceId]/(integrations)/whatsapps/[id]/useful-links/page.tsx
// page: app/space/[workspaceId]/(integrations)/whatsapps/[id]/ecommerce/page.tsx
// page: app/space/[workspaceId]/(integrations)/whatsapps/[id]/automation/page.tsx
// page: app/space/[workspaceId]/(integrations)/whatsapps/[id]/profile/page.tsx
// page: app/space/[workspaceId]/(integrations)/whatsapps/[id]/capi/page.tsx
// page: app/space/[workspaceId]/messengers/[id]/layout.tsx
// page: app/space/[workspaceId]/messengers/[id]/edit/page.tsx
// page: app/space/[workspaceId]/messengers/[id]/message-templates/page.tsx
// page: app/space/[workspaceId]/messengers/[id]/capi/page.tsx
// page: app/space/[workspaceId]/instagrams/[id]/layout.tsx
// page: app/space/[workspaceId]/instagrams/[id]/edit/page.tsx
// page: app/space/[workspaceId]/instagrams/[id]/capi/page.tsx
// page: app/space/[workspaceId]/webchats/[id]/edit/page.tsx
// page: app/space/[workspaceId]/webchats/create/page.tsx
// The channel settings screens. Each layout is
// `resolveGuardedWorkspaceId(params, "superAdmin")`; the data comes from the
// Rust page queries of `rpc_procedures/channels.rs` (never the tokens).
import type { ChannelType } from "@chatbotx.io/database/partials"
import type {
  IntegrationInstagramModel,
  IntegrationMessengerModel,
  IntegrationWebchatModel,
} from "@chatbotx.io/database/types"
import type { WhatsappPhoneNumberDetail } from "@chatbotx.io/integration-whatsapp/api/phone-number"
import type { WhatsappWabaMMLite } from "@chatbotx.io/integration-whatsapp/api/waba"
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@chatbotx.io/ui/components/ui/card"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet, useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { type ReactNode, Suspense, useMemo } from "react"
import { AppBreadcrumb } from "@/components/app-breadcrumb"
import { AiHandoverSettingsCard } from "@/features/integration-ai-handover/components/ai-handover-settings-card"
import type { GetApplyToAllStatusResponse } from "@/features/integration-ai-handover/schema/bulk"
import type { SaveAiHandoverSettingsRequest } from "@/features/integration-ai-handover/schema/request"
import { InstagramSettingTabs } from "@/features/integration-instagram/components/instagram-setting-tabs"
import { UpdateInstagramForm } from "@/features/integration-instagram/components/update-instagram-form"
import { ConversationRoutingCard as MessengerRoutingCard } from "@/features/integration-messenger/components/conversation-routing-card"
import { MessengerSettingTabs } from "@/features/integration-messenger/components/messenger-setting-tabs"
import { MessengerMessageTemplatesTable } from "@/features/integration-messenger/message-templates/message-templates-table"
import { listMessengerMessageTemplatesSearchParamsCache } from "@/features/integration-messenger/message-templates/schema/query"
import { UpdateMessengerForm } from "@/features/integration-messenger/update-messenger-form"
import { CreateWebchatForm } from "@/features/integration-webchat/components/create-webchat-form"
import { UpdateWebchatForm } from "@/features/integration-webchat/components/update-webchat-form"
import { WhatsappAutomationManage } from "@/features/integration-whatsapp/automation/whatsapp-automation-manage"
import { ConversationRoutingCard } from "@/features/integration-whatsapp/components/conversation-routing-card"
import { WhatsappAccountHealths } from "@/features/integration-whatsapp/components/whatsapp-account-healths"
import WhatsappEcommerce from "@/features/integration-whatsapp/components/whatsapp-ecommerce"
import { WhatsappSettingTabs } from "@/features/integration-whatsapp/components/whatsapp-setting-tabs"
import WhatsappUsefulLinks from "@/features/integration-whatsapp/components/whatsapp-useful-links"
import { WhatsappFlowsTable } from "@/features/integration-whatsapp/flows/flows-table"
import { WhatsappMessageTemplatesTable } from "@/features/integration-whatsapp/message-templates/message-templates-table"
import { PaymentSettingsCard } from "@/features/integration-whatsapp/payment/components/payment-settings-card"
import { UpdateWhatsappProfile } from "@/features/integration-whatsapp/profile/update-whatsapp-profile"
import type { IntegrationWhatsappLinkable } from "@/features/integration-whatsapp/queries"
import { WhatsappPhoneVerificationPanel } from "@/features/integration-whatsapp/verification/whatsapp-phone-verification-panel"
import { LockedEditorGuard } from "@/features/templates/components/locked-editor-guard"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import { client } from "@/lib/orpc/orpc"
import { loadPage, serverQuery } from "@/lib/server-action"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"
import { Redirect } from "@/shims/redirect"

function useIds() {
  return useParams({ strict: false }) as { workspaceId: string; id: string }
}

/** A Rust page query of this screen (`serverQuery`). */
function useChannelPage<T>(name: string, input: Record<string, unknown>) {
  const { workspaceId } = useIds()
  return useSuspenseQuery({
    queryKey: ["spa", name, workspaceId, input],
    queryFn: () => serverQuery<T>(name, workspaceId, [workspaceId, input]),
  }).data
}

/** The `(integrations)` channel layout: guard, breadcrumb, tabs. */
function ChannelLayout(props: { label: string; tabs: ReactNode }) {
  const t = useTranslations()
  const { permissions } = useWorkspacePage()
  const { workspaceId, id } = useIds()
  if (!(hasWorkspacePermission(permissions, "superAdmin") && /^\d+$/.test(id))) {
    return <NotFoundPage />
  }
  return (
    <>
      <AppBreadcrumb
        items={[
          {
            label: t("channels.title"),
            href: `/space/${workspaceId}/settings/channels`,
          },
          { label: t(props.label), href: "" },
        ]}
      />
      {props.tabs}
      <div>
        <Outlet />
      </div>
    </>
  )
}

// ---- WhatsApp -----------------------------------------------------------------

export function WhatsappLayout() {
  return (
    <ChannelLayout label="fields.whatsapp.label" tabs={<WhatsappSettingTabs />} />
  )
}

type WhatsappBase = {
  integrationWhatsapp: IntegrationWhatsappLinkable
  urls: {
    analytics: string
    templates: string
    paymentMethods: string
    paymentHistory: string
    ecommerce: string
  }
}

function useWhatsappPage<T>(tab: string) {
  const { id } = useIds()
  return useChannelPage<WhatsappBase & T>("whatsappChannelPage", { id, tab })
}

type WhatsappAutomation = {
  enable_welcome_message: boolean
  prompts: string[]
  commands: { command_name: string; command_description: string }[]
  error?: string
}

export function WhatsappSettingsPage() {
  const t = useTranslations()
  const { workspaceId, id } = useIds()
  const data = useWhatsappPage<{
    automation: WhatsappAutomation
    pixRedirectEnabled: boolean
    handoverResumeFlowId: string | null
    isSuperAdmin: boolean
  }>("settings")
  const promises = useMemo(
    () => Promise.resolve([data.automation] as [WhatsappAutomation]),
    [data.automation],
  )
  return (
    <div className="flex flex-col gap-6">
      <Card>
        <CardHeader>
          <CardTitle>{t("whatsapp.tabs.profile")}</CardTitle>
        </CardHeader>
        <CardContent>
          <UpdateWhatsappProfile workspaceId={workspaceId} />
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>{t("whatsapp.tabs.automation")}</CardTitle>
        </CardHeader>
        <CardContent>
          <WhatsappAutomationManage
            integrationWhatsapp={data.integrationWhatsapp}
            promises={promises as never}
          />
        </CardContent>
      </Card>
      <PaymentSettingsCard
        integrationWhatsappId={id}
        isSuperAdmin={data.isSuperAdmin}
        pixRedirectEnabled={data.pixRedirectEnabled}
        workspaceId={workspaceId}
      />
      <ConversationRoutingCard
        handoverResumeFlowId={data.handoverResumeFlowId}
        integrationWhatsappId={id}
        isSuperAdmin={data.isSuperAdmin}
        workspaceId={workspaceId}
      />
    </div>
  )
}

export function WhatsappAccountHealthsPage() {
  const t = useTranslations("whatsapp.accountHealths")
  const { workspaceId, id } = useIds()
  const data = useWhatsappPage<{
    phoneNumber: WhatsappPhoneNumberDetail | null
    waba: WhatsappWabaMMLite
    businessManagerUrl: string
    displayPhoneNumber: string
    verifiedName: string
    webhookUrl: string
    registrationStatus: string
    registrationError: never
    verificationCodeRequestedAt: string | null
  }>("accountHealths")
  return (
    <>
      {data.registrationStatus !== "registered" && (
        <WhatsappPhoneVerificationPanel
          displayPhoneNumber={data.displayPhoneNumber}
          initialCodeRequestedAt={data.verificationCodeRequestedAt}
          integrationId={id}
          registrationError={data.registrationError}
          verifiedName={data.verifiedName}
          workspaceId={workspaceId}
        />
      )}
      {data.phoneNumber ? (
        <WhatsappAccountHealths
          businessManagerUrl={data.businessManagerUrl}
          phoneNumber={data.phoneNumber}
          waba={data.waba}
          webhookUrl={data.webhookUrl}
        />
      ) : (
        <Card className="my-4">
          <CardContent className="flex flex-col gap-2">
            <h2 className="font-medium text-base">{t("unavailable.title")}</h2>
            <p className="text-muted-foreground text-sm">
              {t("unavailable.description")}
            </p>
          </CardContent>
        </Card>
      )}
    </>
  )
}

export function WhatsappFlowsPage() {
  const data = useWhatsappPage<{ flows: never[] }>("flows")
  const promises = useMemo(() => Promise.resolve(data.flows), [data.flows])
  return (
    <Suspense>
      <WhatsappFlowsTable
        integrationWhatsapp={data.integrationWhatsapp}
        promises={promises}
      />
    </Suspense>
  )
}

export function WhatsappMessageTemplatesPage() {
  const data = useWhatsappPage<{ messageTemplates: never[] }>(
    "messageTemplates",
  )
  const promises = useMemo(
    () => Promise.resolve(data.messageTemplates),
    [data.messageTemplates],
  )
  return (
    <Suspense>
      <WhatsappMessageTemplatesTable
        integrationWhatsapp={data.integrationWhatsapp}
        promises={promises}
      />
    </Suspense>
  )
}

export function WhatsappUsefulLinksPage() {
  return <WhatsappUsefulLinks urls={useWhatsappPage("links").urls} />
}

export function WhatsappEcommercePage() {
  return <WhatsappEcommerce urls={useWhatsappPage("links").urls} />
}

/** `automation`, `profile` → `settings`; `capi` → `ads`. */
function TabRedirect(props: { channel: string; to: string }) {
  const { workspaceId, id } = useIds()
  if (!/^\d+$/.test(id)) {
    return <NotFoundPage />
  }
  return (
    <Redirect href={`/space/${workspaceId}/${props.channel}/${id}/${props.to}`} />
  )
}

export function WhatsappSettingsRedirect() {
  return <TabRedirect channel="whatsapps" to="settings" />
}

export function WhatsappCapiRedirect() {
  return <TabRedirect channel="whatsapps" to="ads" />
}

// ---- Messenger ----------------------------------------------------------------

export function MessengerLayout() {
  return (
    <ChannelLayout
      label="fields.messenger.label"
      tabs={<MessengerSettingTabs />}
    />
  )
}

export function MessengerEditPage() {
  const { workspaceId, id } = useIds()
  const data = useChannelPage<{
    integrationMessenger: IntegrationMessengerModel
    markReadOnOutbound: boolean
    isSuperAdmin: boolean
    canEditAiHandover: boolean
    aiHandover: SaveAiHandoverSettingsRequest
  }>("messengerChannelPage", { id })
  const inboxId = String(data.integrationMessenger.inboxId)
  const { data: applyToAllStatus } = useSuspenseQuery({
    queryKey: ["spa", "aiHandoverApplyToAll", workspaceId, inboxId],
    queryFn: () =>
      client.aiHandoverAPIs.getApplyToAllStatus({
        workspaceId,
        inboxId,
      }) as Promise<GetApplyToAllStatusResponse>,
  })
  return (
    <div className="flex flex-col gap-6">
      <UpdateMessengerForm
        integrationMessenger={data.integrationMessenger}
        markReadOnOutbound={data.markReadOnOutbound}
        workspaceId={workspaceId}
      />
      <MessengerRoutingCard
        handoverResumeFlowId={
          (data.integrationMessenger.handoverResumeFlowId as string | null) ??
          null
        }
        integrationMessengerId={id}
        isSuperAdmin={data.isSuperAdmin}
        workspaceId={workspaceId}
      />
      <AiHandoverSettingsCard
        canEdit={data.canEditAiHandover}
        inboxId={inboxId}
        initialApplyToAllStatus={applyToAllStatus}
        initialValues={data.aiHandover}
        workspaceId={workspaceId}
      />
    </div>
  )
}

export function MessengerMessageTemplatesPage() {
  const { id } = useIds()
  const searchParams = useSearchParams()
  const search = listMessengerMessageTemplatesSearchParamsCache.parse(
    Object.fromEntries(searchParams),
  )
  const data = useChannelPage<{
    integrationMessenger: IntegrationMessengerModel
    channels: { id: string; name: string }[]
    templates: { data: never[]; pageCount: number }
  }>("messengerTemplatesPage", { id, ...search })
  const promises = useMemo(
    () => Promise.resolve(data.templates),
    [data.templates],
  )
  return (
    <Suspense>
      <MessengerMessageTemplatesTable
        channels={data.channels}
        integrationMessenger={data.integrationMessenger}
        promises={promises}
      />
    </Suspense>
  )
}

export function MessengerCapiRedirect() {
  return <TabRedirect channel="messengers" to="ads" />
}

// ---- Instagram ----------------------------------------------------------------

export function InstagramLayout() {
  return (
    <ChannelLayout
      label="fields.instagram.label"
      tabs={<InstagramSettingTabs />}
    />
  )
}

export function InstagramEditPage() {
  const { id } = useIds()
  const data = useChannelPage<{
    integrationInstagram: IntegrationInstagramModel
    markReadOnOutbound: boolean
  }>("instagramChannelPage", { id })
  return (
    <UpdateInstagramForm
      integrationInstagram={data.integrationInstagram}
      markReadOnOutbound={data.markReadOnOutbound}
    />
  )
}

export function InstagramCapiRedirect() {
  return <TabRedirect channel="instagrams" to="ads" />
}

// ---- Webchat ------------------------------------------------------------------

function useSuperAdminGuard() {
  const { permissions } = useWorkspacePage()
  return hasWorkspacePermission(permissions, "superAdmin")
}

function WebchatEditContent() {
  const { workspaceId, id } = useIds()
  const data = useChannelPage<{
    integrationWebchat: IntegrationWebchatModel
    markReadOnOutbound: boolean
  }>("webchatChannelPage", { id })
  return (
    <LockedEditorGuard
      resourceId={id}
      resourceKind="integrationWebchat"
      workspaceId={workspaceId}
    >
      <UpdateWebchatForm
        integrationWebchat={data.integrationWebchat}
        markReadOnOutbound={data.markReadOnOutbound}
      />
    </LockedEditorGuard>
  )
}

export function WebchatEditPage() {
  const { id } = useIds()
  if (!(useSuperAdminGuard() && /^\d+$/.test(id))) {
    return <NotFoundPage />
  }
  return (
    <Suspense fallback={<div>Loading...</div>}>
      <WebchatEditContent />
    </Suspense>
  )
}

function WebchatCreateContent() {
  const { workspaceId } = useIds()
  // `resolveChannelCreatable(workspaceId, "webchat")`.
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "settingsChannels", workspaceId],
    queryFn: () =>
      loadPage<{ creatable: ChannelType[] }>("settingsChannels", {
        workspaceId,
      }),
  })
  if (!data.creatable.includes("webchat")) {
    return <NotFoundPage />
  }
  return <CreateWebchatForm workspaceId={workspaceId} />
}

export function WebchatCreatePage() {
  if (!useSuperAdminGuard()) {
    return <NotFoundPage />
  }
  return (
    <Suspense fallback={<div>Loading...</div>}>
      <WebchatCreateContent />
    </Suspense>
  )
}
