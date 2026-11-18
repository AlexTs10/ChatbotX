// page: app/space/[workspaceId]/templates/page.tsx
// page: app/space/[workspaceId]/templates/create/page.tsx
// page: app/space/[workspaceId]/templates/[id]/edit/page.tsx
// page: app/space/[workspaceId]/templates/installs/page.tsx
// page: app/space/[workspaceId]/templates/installs/[id]/page.tsx
// page: app/t/[shareToken]/layout.tsx
// page: app/t/[shareToken]/page.tsx
// page: app/t/[shareToken]/install/page.tsx
// page: app/admin/default-template/page.tsx
// The template screens. The page reads are template actions of the Rust
// server (`crates/api/src/rpc/template_actions.rs`: manage reads need the
// `flows` permission, install reads `canViewTemplateInstalls`), the share
// pages are `pages.templateShare` / `pages.templateInstall`, the admin page
// `getRootDefaultTemplateSettings` (platform super admin).
import type {
  TemplateInstallationDetail as Detail,
  TemplateInstallationListItem,
  TemplateInstallationResourcePage,
  TemplateListItem,
  TemplateSyncHistoryItem,
  TemplateVersionHistoryItem,
} from "@chatbotx.io/business"
import {
  type TemplateInstallationStatus,
  templateInstallationStatuses,
} from "@chatbotx.io/database/partials"
import { Button } from "@chatbotx.io/ui/components/ui/button"
import { Card, CardContent } from "@chatbotx.io/ui/components/ui/card"
import { ORPCError } from "@orpc/client"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet, useParams } from "@tanstack/react-router"
import Image from "next/image"
import Link from "next/link"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { Suspense } from "react"
import { AppBreadcrumb } from "@/components/app-breadcrumb"
import { AppTab } from "@/components/app-tab"
import { PublicMessage } from "@/components/public-message"
import { ThemeSwitcher } from "@/components/theme-switcher"
import { DefaultTemplateSettings } from "@/features/platform-default-template/default-template-settings"
import { CreateTemplateDialog } from "@/features/templates/components/create-template-dialog"
import { CreateWorkspaceFromTemplateLink } from "@/features/templates/components/create-workspace-from-template-link"
import { PublicTemplateCard } from "@/features/templates/components/public-template-card"
import { PublicTemplateHeading } from "@/features/templates/components/public-template-heading"
import { TemplateForm } from "@/features/templates/components/template-form"
import { TemplateInstallationDetail } from "@/features/templates/components/template-installation-detail"
import { TemplateInstallsTable } from "@/features/templates/components/template-installs-table"
import { TemplateVersionsList } from "@/features/templates/components/template-versions-list"
import { TemplatesTable } from "@/features/templates/components/templates-table"
import { TemplatesTabs } from "@/features/templates/components/templates-tabs"
import { WorkspacePicker } from "@/features/templates/components/workspace-picker"
import {
  canViewTemplateInstalls,
  listSelectableTemplateCategories,
  TEMPLATE_MANAGE_PERMISSION,
} from "@/features/templates/lib/permissions"
import { useTenantSettings } from "@/features/tenant/tenant-settings-provider"
import { getWorkspaceLogoUrl } from "@/features/workspaces/helpers"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import { loadPage, serverFunction, serverQuery } from "@/lib/server-action"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"
import { Redirect } from "@/shims/redirect"
import { getPublicFileUrl } from "@chatbotx.io/utils"

function useIds() {
  return useParams({ strict: false }) as { workspaceId: string; id?: string }
}

/** A template read; `null` = 404 (the page's `notFound()`). */
function useTemplateRead<T>(name: string, args: unknown[]): T | null {
  const { workspaceId } = useIds()
  return useSuspenseQuery({
    queryKey: ["spa", "templates", name, workspaceId, args],
    queryFn: async () => {
      try {
        return await serverQuery<T>(name, workspaceId, [workspaceId, ...args])
      } catch (error) {
        if (error instanceof ORPCError && error.status === 404) {
          return null
        }
        throw error
      }
    },
  }).data
}

function Breadcrumb(props: { items: { label: string; href: string }[] }) {
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

/** `requireWorkspacePermission(workspaceId, TEMPLATE_MANAGE_PERMISSION)`. */
function useCanManage() {
  return hasWorkspacePermission(
    useWorkspacePage().permissions,
    TEMPLATE_MANAGE_PERMISSION,
  )
}

/** `templateService.list`: every page of `templates.list`. */
async function listAllTemplates(workspaceId: string) {
  const items: TemplateListItem[] = []
  let cursor: string | null = null
  do {
    const page: { items: TemplateListItem[]; nextCursor: string | null } =
      await serverQuery("listTemplates", workspaceId, [
        workspaceId,
        cursor ? { cursor } : {},
      ])
    items.push(...page.items)
    cursor = page.nextCursor
  } while (cursor)
  return items
}

function TemplatesList() {
  const { workspaceId } = useIds()
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "templates", "listTemplates", workspaceId],
    queryFn: () => listAllTemplates(workspaceId),
  })
  return <TemplatesTable templates={data} workspaceId={workspaceId} />
}

export function TemplatesPage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const { permissions } = useWorkspacePage()
  if (!useCanManage()) {
    return <NotFoundPage />
  }
  return (
    <div className="flex flex-col gap-4">
      <Breadcrumb items={[{ label: t("templates.title"), href: "" }]} />
      <TemplatesTabs
        active="mine"
        permissions={permissions}
        workspaceId={workspaceId}
      />
      <div className="flex justify-end">
        <CreateTemplateDialog workspaceId={workspaceId} />
      </div>
      <Suspense>
        <TemplatesList />
      </Suspense>
    </div>
  )
}

export function CreateTemplatePage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const { permissions } = useWorkspacePage()
  const name = useSearchParams().get("name") ?? ""
  if (!useCanManage()) {
    return <NotFoundPage />
  }
  // The name typed in the "Create" dialog; anything unusable starts blank.
  const initialName = name.trim().length <= 255 ? name.trim() : ""
  return (
    <div className="flex flex-col gap-4">
      <Breadcrumb
        items={[
          {
            label: t("templates.title"),
            href: `/space/${workspaceId}/templates`,
          },
          { label: t("actions.create"), href: "" },
        ]}
      />
      <TemplateForm
        initialName={initialName}
        selectableCategories={listSelectableTemplateCategories(permissions)}
        workspaceId={workspaceId}
      />
    </div>
  )
}

function TemplateVersions(props: { currentVersion: number }) {
  const { id } = useIds()
  const versions = useTemplateRead<TemplateVersionHistoryItem[]>(
    "listTemplateVersions",
    [id],
  )
  return (
    <TemplateVersionsList
      currentVersion={props.currentVersion}
      versions={versions ?? []}
    />
  )
}

function EditTemplate() {
  const t = useTranslations()
  const { workspaceId, id } = useIds()
  const { permissions } = useWorkspacePage()
  const showVersions = useSearchParams().get("tab") === "versions"
  // biome-ignore lint/suspicious/noExplicitAny: the detail type lives on the server
  const template = useTemplateRead<any>("getEditableTemplate", [id])
  if (!template) {
    return <NotFoundPage />
  }
  const editHref = `/space/${workspaceId}/templates/${id}/edit`
  return (
    <div className="flex flex-col gap-4">
      <Breadcrumb
        items={[
          {
            label: t("templates.title"),
            href: `/space/${workspaceId}/templates`,
          },
          { label: t("actions.edit"), href: "" },
        ]}
      />
      <AppTab
        tabs={[
          {
            label: t("templates.versions.detailsTab"),
            href: editHref,
            isActive: !showVersions,
          },
          {
            label: t("templates.versions.tab"),
            href: `${editHref}?tab=versions`,
            isActive: showVersions,
          },
        ]}
      />
      {showVersions ? (
        <Suspense>
          <TemplateVersions currentVersion={template.version} />
        </Suspense>
      ) : (
        <TemplateForm
          selectableCategories={listSelectableTemplateCategories(permissions)}
          template={template}
          workspaceId={workspaceId}
        />
      )}
    </div>
  )
}

export function EditTemplatePage() {
  if (!useCanManage()) {
    return <NotFoundPage />
  }
  return <EditTemplate />
}

/** `requireTemplateInstallsAccess`. */
export function TemplateInstallsLayout() {
  if (!canViewTemplateInstalls(useWorkspacePage().permissions)) {
    return <NotFoundPage />
  }
  return <Outlet />
}

// Snowflake ids are digit strings; anything else is a forged cursor.
const CURSOR_PATTERN = /^\d{1,20}$/

type InstallationsPage = {
  items: TemplateInstallationListItem[]
  nextCursor: string | null
}

function TemplateInstalls() {
  const { workspaceId } = useIds()
  const searchParams = useSearchParams()
  const parsedStatus = templateInstallationStatuses.safeParse(
    searchParams.get("status") ?? undefined,
  )
  const status: TemplateInstallationStatus | null = parsedStatus.success
    ? parsedStatus.data
    : null
  const cursor = searchParams.get("cursor") ?? ""
  const page = useTemplateRead<InstallationsPage>("listTemplateInstallations", [
    {
      ...(status ? { status } : {}),
      ...(CURSOR_PATTERN.test(cursor) ? { cursor } : {}),
    },
  ])
  // An install running under another filter or page must still keep the
  // list refreshing.
  const inFlight = useTemplateRead<InstallationsPage>(
    "listTemplateInstallations",
    [{ activeOnly: true, limit: 1 }],
  )
  return (
    <TemplateInstallsTable
      hasActiveInstalls={(inFlight?.items.length ?? 0) > 0}
      installations={page?.items ?? []}
      nextCursor={page?.nextCursor ?? null}
      status={status}
      workspaceId={workspaceId}
    />
  )
}

export function TemplateInstallsPage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const { permissions } = useWorkspacePage()
  return (
    <div className="flex flex-col gap-4">
      <Breadcrumb
        items={[
          {
            label: t("templates.title"),
            href: `/space/${workspaceId}/templates`,
          },
          { label: t("templates.tabs.installed"), href: "" },
        ]}
      />
      <TemplatesTabs
        active="installed"
        permissions={permissions}
        workspaceId={workspaceId}
      />
      <Suspense>
        <TemplateInstalls />
      </Suspense>
    </div>
  )
}

function InstallationResources(props: {
  detail: Detail
  activeCategory: Detail["resourceSummary"][number]["category"] | undefined
}) {
  const { workspaceId, id } = useIds()
  const empty = { items: [], nextCursor: null }
  const resources = useTemplateRead<TemplateInstallationResourcePage>(
    "listTemplateInstallationResources",
    [id, props.activeCategory ? { category: props.activeCategory } : {}],
  )
  const history = useTemplateRead<TemplateSyncHistoryItem[]>(
    "listTemplateSyncHistory",
    [id],
  )
  return (
    <TemplateInstallationDetail
      activeCategory={props.activeCategory}
      detail={props.detail}
      history={history ?? []}
      resources={props.activeCategory ? (resources ?? empty) : empty}
      workspaceId={workspaceId}
    />
  )
}

export function TemplateInstallationPage() {
  const t = useTranslations()
  const { workspaceId, id } = useIds()
  const category = useSearchParams().get("category")
  const detail = useTemplateRead<Detail>("getTemplateInstallation", [id])
  if (!detail) {
    return <NotFoundPage />
  }
  const activeCategory =
    detail.resourceSummary.find((row) => row.category === category)?.category ??
    detail.resourceSummary[0]?.category
  return (
    <div className="flex flex-col gap-4">
      <Breadcrumb
        items={[
          {
            label: t("templates.title"),
            href: `/space/${workspaceId}/templates`,
          },
          {
            label: t("templates.tabs.installed"),
            href: `/space/${workspaceId}/templates/installs`,
          },
          { label: detail.templateName, href: "" },
        ]}
      />
      <Suspense>
        <InstallationResources activeCategory={activeCategory} detail={detail} />
      </Suspense>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Share pages: `/t/{key}` (older links) and `/t/template?id=…&key=…` (the
// builder's next.config rewrite of the current link).
// ---------------------------------------------------------------------------

type PublicTemplate = Parameters<typeof PublicTemplateCard>[0]["template"]
type SharePage = { template: PublicTemplate | null; isAuthenticated: boolean }

function useShareToken() {
  const { shareToken } = useParams({ strict: false }) as { shareToken: string }
  const key = useSearchParams().get("key")
  return shareToken === "template" && key ? key : shareToken
}

function useSharePage() {
  const shareToken = useShareToken()
  return useSuspenseQuery({
    queryKey: ["spa", "pages.templateShare", shareToken],
    queryFn: () => loadPage<SharePage>("templateShare", { shareToken }),
  }).data
}

/** The template's cover, or the tenant's icon when it has none. */
function useTemplateImageUrl(imageUrl: string | null) {
  const { storageUrl, faviconUrl } = useTenantSettings()
  return imageUrl ? getPublicFileUrl(imageUrl, storageUrl) : faviconUrl
}

function SignInPrompt(props: { shareToken: string; label: string }) {
  return (
    <Button
      nativeButton={false}
      render={
        <a
          href={`/auth/sign-in?callbackURL=${encodeURIComponent(`/t/${props.shareToken}`)}`}
        >
          {props.label}
        </a>
      }
      variant="ghost"
    />
  )
}

export function PublicTemplateLayout() {
  const t = useTranslations("auth")
  const { name, logoLightUrl, logoDarkUrl } = useTenantSettings()
  const shareToken = useShareToken()
  const { isAuthenticated } = useSharePage()
  return (
    <div className="flex min-h-svh flex-col bg-muted">
      <header className="w-full border-b bg-background">
        <div className="flex h-14 items-center justify-between px-4 md:px-6">
          <div className="flex items-center">
            <Image
              alt={name}
              className="block h-8 w-auto dark:hidden"
              height={32}
              src={logoDarkUrl}
              width={108}
            />
            <Image
              alt={name}
              className="hidden h-8 w-auto dark:block"
              height={32}
              src={logoLightUrl}
              width={108}
            />
          </div>
          {isAuthenticated ? null : (
            <SignInPrompt label={t("signIn")} shareToken={shareToken} />
          )}
        </div>
      </header>
      <div className="mx-auto flex w-full max-w-4xl flex-1 flex-col gap-6 p-4 md:py-12">
        <Outlet />
      </div>
      <div className="fixed inset-e-2 bottom-2">
        <ThemeSwitcher />
      </div>
    </div>
  )
}

function InvalidLink() {
  const t = useTranslations("templatesPublicPage")
  return (
    <PublicMessage
      description={t("invalidDescription")}
      title={t("invalidTitle")}
    />
  )
}

function PublicTemplate(props: { template: PublicTemplate }) {
  const t = useTranslations("templatesPublicPage")
  const shareToken = useShareToken()
  const id = useSearchParams().get("id")
  const imageUrl = useTemplateImageUrl(props.template.imageUrl)
  // The install page's link names the template like this one does.
  const installQuery = new URLSearchParams({
    ...(id ? { id } : {}),
    key: shareToken,
  })
  return (
    <main className="w-full">
      <PublicTemplateCard
        imageUrl={imageUrl}
        install={
          <Button
            nativeButton={false}
            render={<Link href={`/t/template/install?${installQuery}`} />}
          >
            {t("install")}
          </Button>
        }
        template={props.template}
      />
    </main>
  )
}

/**
 * Every failure mode (bad token, disabled share, expired) is the same
 * generic invalid-link message: the page is never a token-existence oracle.
 */
export function PublicTemplatePage() {
  const { template } = useSharePage()
  return template ? <PublicTemplate template={template} /> : <InvalidLink />
}

type InstallPage = {
  workspaces: {
    id: string
    name: string
    logo: string | null
    crossTenant: boolean
  }[]
  requiredInboxes: Parameters<typeof WorkspacePicker>[0]["requiredInboxes"]
  inboxOptions: Parameters<typeof WorkspacePicker>[0]["inboxOptions"]
  installedCopies: Parameters<typeof WorkspacePicker>[0]["installedCopies"]
}

/** `InstallSection`: choose the workspace to install into. */
function InstallSection(props: { shareToken: string }) {
  const t = useTranslations("templatesPublicPage")
  const { storageUrl } = useTenantSettings()
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "pages.templateInstall", props.shareToken],
    queryFn: () =>
      loadPage<InstallPage>("templateInstall", {
        shareToken: props.shareToken,
      }),
  })
  const createLink = (
    <CreateWorkspaceFromTemplateLink
      label={t("createWorkspace")}
      shareToken={props.shareToken}
    />
  )
  if (data.workspaces.length === 0) {
    return (
      <div className="flex flex-col items-center gap-3">
        <p className="text-muted-foreground text-sm">
          {t("noEligibleWorkspaces")}
        </p>
        {createLink}
      </div>
    )
  }
  return (
    <div className="flex flex-col items-center gap-6">
      <WorkspacePicker
        inboxOptions={data.inboxOptions}
        installedCopies={data.installedCopies}
        requiredInboxes={data.requiredInboxes}
        shareToken={props.shareToken}
        workspaces={data.workspaces.map((workspace) => ({
          id: workspace.id,
          name: workspace.name,
          crossTenant: workspace.crossTenant,
          logoUrl: getWorkspaceLogoUrl(workspace, storageUrl) ?? null,
        }))}
      />
      {createLink}
    </div>
  )
}

function PublicTemplateInstall(props: { template: PublicTemplate }) {
  const shareToken = useShareToken()
  const imageUrl = useTemplateImageUrl(props.template.imageUrl)
  return (
    <main className="flex w-full flex-col gap-8">
      <Card>
        <CardContent>
          <PublicTemplateHeading imageUrl={imageUrl} template={props.template} />
        </CardContent>
      </Card>
      <Suspense>
        <InstallSection shareToken={shareToken} />
      </Suspense>
    </main>
  )
}

/** Where the share page's Install leads; a signed-out viewer signs in first. */
export function PublicTemplateInstallPage() {
  const shareToken = useShareToken()
  const { template, isAuthenticated } = useSharePage()
  if (!template) {
    return <InvalidLink />
  }
  if (!isAuthenticated) {
    return (
      <Redirect
        href={`/auth/sign-in?callbackURL=${encodeURIComponent(`/t/${shareToken}/install`)}`}
      />
    )
  }
  return <PublicTemplateInstall template={template} />
}

// ---------------------------------------------------------------------------
// `/admin/default-template`
// ---------------------------------------------------------------------------

const getRootDefaultTemplateSettings = serverFunction(
  "getRootDefaultTemplateSettings",
)

function RootDefaultTemplate() {
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "getRootDefaultTemplateSettings"],
    queryFn: async () => {
      try {
        return { settings: await getRootDefaultTemplateSettings() }
      } catch (error) {
        if (error instanceof ORPCError && error.status === 404) {
          return null
        }
        throw error
      }
    },
  })
  if (!data) {
    return <NotFoundPage />
  }
  return <DefaultTemplateSettings scope="platform" settings={data.settings} />
}

export function AdminDefaultTemplatePage() {
  const t = useTranslations()
  return (
    <div className="space-y-4">
      <h3 className="font-bold text-lg sm:text-xl">
        {t("platformDefaultTemplate.title")}
      </h3>
      <Suspense>
        <RootDefaultTemplate />
      </Suspense>
    </div>
  )
}
