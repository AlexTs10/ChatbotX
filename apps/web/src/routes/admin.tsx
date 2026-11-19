// page: app/admin/layout.tsx
// page: app/admin/page.tsx
// page: app/admin/workspaces/page.tsx
// page: app/admin/(enterprise)/layout.tsx
// page: app/admin/(enterprise)/admins/page.tsx
// page: app/admin/(enterprise)/help-items/page.tsx
// page: app/admin/(enterprise)/audit-logs/page.tsx
// page: app/admin/(enterprise)/(non-cloud)/layout.tsx
// page: app/admin/(enterprise)/(non-cloud)/branding/page.tsx
// page: app/admin/(enterprise)/(non-cloud)/smtp/page.tsx
// page: app/admin/(enterprise)/(non-cloud)/email-templates/page.tsx
// page: app/admin/(enterprise)/(non-cloud)/email-templates/[type]/page.tsx
// page: app/admin/platform-channels/page.tsx
// page: app/admin/platform-credentials/page.tsx
// page: app/admin/workspaces/deleted/page.tsx
import type { SmtpCredentialPublic } from "@chatbotx.io/database/partials"
import type { TenantHelpItemModel } from "@chatbotx.io/database/types"
import { ORPCError } from "@orpc/client"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { type ReactNode, Suspense, useMemo } from "react"
import { PlatformAuditLogsTable } from "@/enterprise/features/platform-audit-logs/platform-audit-logs-table"
import { listPlatformAuditLogsSearchParamsCache } from "@/enterprise/features/platform-audit-logs/schema/query"
import { PlatformSmtpForm } from "@/enterprise/features/platform-smtp/platform-smtp-form"
import { PlatformAdminsTable } from "@/enterprise/features/platform-admins/platform-admins-table"
import { getPlatformAdminsSearchParamsCache } from "@/enterprise/features/platform-admins/query"
import { AdminSidebar } from "@/features/admin/components/admin-sidebar"
import { AdminWorkspacesTable } from "@/features/admin-workspaces/admin-workspaces-table"
import { DeletedWorkspacesTable } from "@/features/admin-workspaces/deleted-workspaces-table"
import { listAdminWorkspaces } from "@/features/admin-workspaces/queries"
import { getDeletedWorkspacesSearchParamsCache } from "@/features/admin-workspaces/schema/deleted-query"
import { getAdminWorkspacesSearchParamsCache } from "@/features/admin-workspaces/schema/query"
import { ManageHelpItems } from "@/features/help-items/manage-help-items"
import { ManageLayout } from "@/features/manage/manage-layout"
import { isCloud } from "@/env"
import { loadPage, serverFunction } from "@/lib/server-action"
import {
  BrandingPage,
  EmailTemplatePage,
  EmailTemplatesPage,
  PlatformChannelsPage,
  PlatformCredentialsPage,
} from "@/routes/platform-console"
import { NotFoundPage } from "@/routes/root"
import { Redirect } from "@/shims/redirect"

type AdminPage = { showEnterpriseItems: boolean; consentRequired: boolean }

/** `pages.admin`: `null` = not a platform admin (the layout's `notFound()`). */
function useAdminPage(): AdminPage | null {
  return useSuspenseQuery({
    queryKey: ["spa", "pages.admin"],
    queryFn: async () => {
      try {
        return await loadPage<AdminPage>("admin")
      } catch (error) {
        if (error instanceof ORPCError && error.status === 404) {
          return null
        }
        throw error
      }
    },
  }).data
}

export function AdminLayout() {
  const page = useAdminPage()
  if (!page) {
    return <NotFoundPage />
  }
  return (
    <ManageLayout
      sidebar={<AdminSidebar showEnterpriseItems={page.showEnterpriseItems} />}
    >
      <Outlet />
    </ManageLayout>
  )
}

export const AdminIndexPage = () => (
  <Redirect href="/admin/platform-credentials" />
)

function Section(props: {
  title: string
  description: string
  children: ReactNode
}) {
  return (
    <div className="space-y-4">
      <div className="space-y-1">
        <h3 className="font-bold text-lg sm:text-xl">{props.title}</h3>
        <p className="text-muted-foreground text-sm">{props.description}</p>
      </div>
      <Suspense>{props.children}</Suspense>
    </div>
  )
}

/** The search of a page, parsed once per search string. */
function useSearch<S>(cache: {
  all: () => S
  parse: (input: Record<string, string>) => unknown
}) {
  const searchParams = useSearchParams()
  const key = searchParams.toString()
  // biome-ignore lint/correctness/useExhaustiveDependencies: the search string is the key
  return useMemo(
    () => cache.parse(Object.fromEntries(searchParams)) as S,
    [key],
  )
}

export function AdminWorkspacesPage() {
  const t = useTranslations()
  const page = useAdminPage()
  const search = useSearch(getAdminWorkspacesSearchParamsCache)
  const promises = useMemo(
    () => Promise.all([listAdminWorkspaces(search)]),
    [search],
  )
  return (
    <Section
      description={t(
        (page?.consentRequired ?? true)
          ? "platformAdmin.workspaces.description"
          : "platformAdmin.workspaces.descriptionConsentDisabled",
      )}
      title={t("platformAdmin.workspaces.title")}
    >
      <AdminWorkspacesTable
        canCreateWorkspace={page?.showEnterpriseItems ?? false}
        consentRequired={page?.consentRequired ?? true}
        promises={promises}
      />
    </Section>
  )
}

/** `(enterprise)/layout.tsx`: 404 without the enterprise features. */
function Enterprise(props: { children: ReactNode }) {
  const page = useAdminPage()
  return page?.showEnterpriseItems ? props.children : <NotFoundPage />
}

const listPlatformAdmins = serverFunction("listPlatformAdmins")
const listRootHelpItems = serverFunction("listRootHelpItems")

function PlatformAdmins() {
  const t = useTranslations()
  const search = useSearch(getPlatformAdminsSearchParamsCache)
  const promises = useMemo(
    () => Promise.all([listPlatformAdmins(search.page)]),
    [search.page],
  )
  return (
    <Section
      description={t("platformAdmin.admins.description")}
      title={t("platformAdmin.admins.title")}
    >
      <PlatformAdminsTable page={search.page} promises={promises} />
    </Section>
  )
}

export const AdminAdminsPage = () => (
  <Enterprise>
    <PlatformAdmins />
  </Enterprise>
)

function HelpItems() {
  const t = useTranslations()
  const { data: items } = useSuspenseQuery({
    queryKey: ["spa", "listRootHelpItems"],
    queryFn: () => listRootHelpItems() as Promise<TenantHelpItemModel[]>,
  })
  return (
    <Section
      description={t("platformAdmin.helpItems.description")}
      title={t("platformAdmin.helpItems.title")}
    >
      <ManageHelpItems items={items} scope="platform" />
    </Section>
  )
}

export const AdminHelpItemsPage = () => (
  <Enterprise>
    <HelpItems />
  </Enterprise>
)

const listPlatformAuditLogs = serverFunction("listPlatformAuditLogs")
const listPlatformAuditWorkspaces = serverFunction(
  "listPlatformAuditWorkspaces",
)

function PlatformAuditLogs() {
  const t = useTranslations()
  const search = useSearch(listPlatformAuditLogsSearchParamsCache)
  const promises = useMemo(
    () =>
      Promise.all([
        listPlatformAuditLogs(search),
        listPlatformAuditWorkspaces(search.workspaceId || undefined),
      ]),
    [search],
  )
  return (
    <Section
      description={t("platformAdmin.auditLogs.description")}
      title={t("platformAdmin.auditLogs.title")}
    >
      <PlatformAuditLogsTable promises={promises} search={search} />
    </Section>
  )
}

export const AdminAuditLogsPage = () => (
  <Enterprise>
    <PlatformAuditLogs />
  </Enterprise>
)

/** `(enterprise)/(non-cloud)/layout.tsx`: 404 on the cloud edition. */
function NonCloud(props: { children: ReactNode }) {
  return (
    <Enterprise>{isCloud() ? <NotFoundPage /> : props.children}</Enterprise>
  )
}

export const AdminBrandingPage = () => (
  <NonCloud>
    <BrandingPage platform />
  </NonCloud>
)

export const AdminEmailTemplatesPage = () => (
  <NonCloud>
    <EmailTemplatesPage platform />
  </NonCloud>
)

export const AdminEmailTemplatePage = () => (
  <NonCloud>
    <EmailTemplatePage platform />
  </NonCloud>
)

const getPlatformSmtpSettings = serverFunction("getPlatformSmtpSettings")

function PlatformSmtp() {
  const { data: settings } = useSuspenseQuery({
    queryKey: ["spa", "getPlatformSmtpSettings"],
    queryFn: () =>
      getPlatformSmtpSettings() as Promise<SmtpCredentialPublic | null>,
  })
  return <PlatformSmtpForm settings={settings} />
}

export function AdminSmtpPage() {
  const t = useTranslations()
  return (
    <NonCloud>
      <Section
        description={t("platformAdmin.smtp.description")}
        title={t("platformAdmin.smtp.title")}
      >
        <PlatformSmtp />
      </Section>
    </NonCloud>
  )
}

export const AdminPlatformChannelsPage = () => (
  <PlatformChannelsPage platform />
)

export const AdminPlatformCredentialsPage = () => (
  <PlatformCredentialsPage platform />
)

const listDeletedWorkspaces = serverFunction("listDeletedWorkspaces")

export function AdminDeletedWorkspacesPage() {
  const t = useTranslations()
  const search = useSearch(getDeletedWorkspacesSearchParamsCache)
  const promises = useMemo(
    () =>
      Promise.all([
        listDeletedWorkspaces({ ...search, search: search.search ?? undefined }),
      ]),
    [search],
  )
  return (
    <Section
      description={t("platformAdmin.workspaces.deletedDescription")}
      title={t("platformAdmin.workspaces.deletedTitle")}
    >
      <DeletedWorkspacesTable promises={promises} />
    </Section>
  )
}
