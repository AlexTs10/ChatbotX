// page: app/admin/layout.tsx
// page: app/admin/page.tsx
// page: app/admin/workspaces/page.tsx
// page: app/admin/(enterprise)/layout.tsx
// page: app/admin/(enterprise)/admins/page.tsx
// page: app/admin/(enterprise)/help-items/page.tsx
import type { TenantHelpItemModel } from "@chatbotx.io/database/types"
import { ORPCError } from "@orpc/client"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { type ReactNode, Suspense, useMemo } from "react"
import { PlatformAdminsTable } from "@/enterprise/features/platform-admins/platform-admins-table"
import { getPlatformAdminsSearchParamsCache } from "@/enterprise/features/platform-admins/query"
import { AdminSidebar } from "@/features/admin/components/admin-sidebar"
import { AdminWorkspacesTable } from "@/features/admin-workspaces/admin-workspaces-table"
import { listAdminWorkspaces } from "@/features/admin-workspaces/queries"
import { getAdminWorkspacesSearchParamsCache } from "@/features/admin-workspaces/schema/query"
import { ManageHelpItems } from "@/features/help-items/manage-help-items"
import { ManageLayout } from "@/features/manage/manage-layout"
import { loadPage, serverFunction } from "@/lib/server-action"
import { NotAvailablePage, NotFoundPage } from "@/routes/root"
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

// ponytail: the other console pages read data the Rust server has no
// procedure for yet (platform credentials / channels, default template,
// deletion records, platform audit logs, SMTP, branding, email templates).
export const AdminNotAvailablePage = () => (
  <NotAvailablePage title="Platform admin" />
)
