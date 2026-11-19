// page: app/manage/layout.tsx
// page: app/manage/page.tsx
// page: app/manage/(enterprise)/layout.tsx
// page: app/manage/(enterprise)/branding/page.tsx
// page: app/manage/(enterprise)/email-templates/page.tsx
// page: app/manage/(enterprise)/email-templates/[type]/page.tsx
// page: app/manage/default-template/layout.tsx
// page: app/manage/default-template/page.tsx
// page: app/manage/help-items/layout.tsx
// page: app/manage/help-items/page.tsx
// page: app/manage/platform-channels/layout.tsx
// page: app/manage/platform-channels/page.tsx
// page: app/manage/platform-credentials/page.tsx
//
// The cloud reseller console. `getManageAccess` (Rust) is the layout's
// guard: cloud edition and `hasOwnCredentialAccess`, else 404. The reseller
// pages' reads also `requireActiveReseller()`.
import type { TenantHelpItemModel } from "@chatbotx.io/database/types"
import type { PortalSaasFlags } from "@chatbotx.io/ui/config/portal-nav"
import { buildResellerPricingUrl } from "@chatbotx.io/ui/lib/portal-pricing-url"
import { Outlet } from "@tanstack/react-router"
import { useTranslations } from "next-intl"
import { type ComponentProps, Suspense } from "react"
import { PortalManageSidebar } from "@/enterprise/features/manage/components/portal-manage-sidebar"
import { ManageHelpItems } from "@/features/help-items/manage-help-items"
import { ManageLayout } from "@/features/manage/manage-layout"
import { DefaultTemplateSettings } from "@/features/platform-default-template/default-template-settings"
import {
  BrandingPage,
  EmailTemplatePage,
  EmailTemplatesPage,
  PlatformChannelsPage,
  PlatformCredentialsPage,
  useConsoleRead,
} from "@/routes/platform-console"
import { NotFoundPage } from "@/routes/root"
import { Redirect } from "@/shims/redirect"

type ManageAccess = {
  canManageCredentials: boolean
  canManageReseller: boolean
  flags: PortalSaasFlags
  /** The white-label reseller's active custom domain (`null`: no pricing). */
  pricing: { domain: string | null } | null
}

export function ManageLayoutPage() {
  const read = useConsoleRead<ManageAccess>("getManageAccess")
  if (!read) {
    return <NotFoundPage />
  }
  const access = read.value
  const domain = access.pricing?.domain
  return (
    <ManageLayout
      sidebar={
        <PortalManageSidebar
          canManageReseller={access.canManageReseller}
          flags={access.flags}
          pricing={
            access.pricing
              ? domain
                ? { state: "active", url: buildResellerPricingUrl(domain) }
                : { state: "missing" }
              : undefined
          }
        />
      }
    >
      <Outlet />
    </ManageLayout>
  )
}

export const ManageIndexPage = () => (
  <Redirect href="/manage/platform-credentials" />
)

export const ManageBrandingPage = () => <BrandingPage platform={false} />

export const ManageEmailTemplatesPage = () => (
  <EmailTemplatesPage platform={false} />
)

export const ManageEmailTemplatePage = () => (
  <EmailTemplatePage platform={false} />
)

export const ManagePlatformChannelsPage = () => (
  <PlatformChannelsPage platform={false} />
)

export const ManagePlatformCredentialsPage = () => (
  <PlatformCredentialsPage platform={false} />
)

function HelpItems() {
  const read = useConsoleRead<TenantHelpItemModel[]>("listOwnHelpItems")
  return read ? (
    <ManageHelpItems items={read.value} scope="tenant" />
  ) : (
    <NotFoundPage />
  )
}

export function ManageHelpItemsPage() {
  const t = useTranslations()
  return (
    <div className="space-y-4">
      <h3 className="font-bold text-lg sm:text-xl">{t("helpItems.title")}</h3>
      <Suspense>
        <HelpItems />
      </Suspense>
    </div>
  )
}

function DefaultTemplate() {
  const read = useConsoleRead<
    ComponentProps<typeof DefaultTemplateSettings>["settings"]
  >("getOwnDefaultTemplateSettings")
  return read ? (
    <DefaultTemplateSettings scope="tenant" settings={read.value} />
  ) : (
    <NotFoundPage />
  )
}

export function ManageDefaultTemplatePage() {
  const t = useTranslations()
  return (
    <div className="space-y-4">
      <h3 className="font-bold text-lg sm:text-xl">
        {t("platformDefaultTemplate.title")}
      </h3>
      <Suspense>
        <DefaultTemplate />
      </Suspense>
    </div>
  )
}
