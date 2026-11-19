// page: app/space/[workspaceId]/contacts/import/page.tsx
// page: app/space/[workspaceId]/contacts/import/histories/page.tsx
// page: app/space/[workspaceId]/contacts/scan/page.tsx
// page: app/space/[workspaceId]/contacts/scan/histories/page.tsx
// page: app/(no-sidebar)/space/[workspaceId]/flows/import/histories/page.tsx
// `requireContactsAccess` is the Rust page loader `pages.contactsAccess`,
// `requireContactScanPageAccess` is `pages.contactScanAccess` (both 404).
// The history queries re-check the same gates in Rust.
import { importTypes } from "@chatbotx.io/database/partials"
import { buttonVariants } from "@chatbotx.io/ui/components/ui/button"
import { useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import { ArrowLeftIcon } from "lucide-react"
import Link from "next/link"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { type ReactNode, Suspense } from "react"
import { ContactScanForm } from "@/features/contact-scan/components/contact-scan-form"
import { ContactScanHistoryTable } from "@/features/contact-scan/components/contact-scan-history-table"
import { listContactScanHistory } from "@/features/contact-scan/queries/list-contact-scan-history.queries"
import { listContactScanHistorySearchParamsCache } from "@/features/contact-scan/schema/query"
import { ImportContactsForm } from "@/features/contacts/import-contact-form"
import { ImportForm } from "@/features/import/components/import-form"
import { ImportHistoryTable } from "@/features/import/components/import-history-table"
import { listImports } from "@/features/import/queries/list-imports.queries"
import { listImportsSearchParamsCache } from "@/features/import/schema/query"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import { useQueryPromise } from "@/lib/query/use-query-promise"
import { loadPage } from "@/lib/server-action"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"

function useWorkspaceId() {
  return (useParams({ strict: false }) as { workspaceId: string }).workspaceId
}

/** The page loader gate: it throws (the not-found page) when denied. */
function Gate(props: {
  page: "contactsAccess" | "contactScanAccess"
  children: ReactNode
}) {
  const workspaceId = useWorkspaceId()
  useSuspenseQuery({
    queryKey: ["spa", props.page, workspaceId],
    queryFn: () => loadPage(props.page, { workspaceId }),
  })
  return props.children
}

function HistoryPage(props: {
  backHref: string
  title: string
  children: ReactNode
}) {
  const t = useTranslations()
  return (
    <div className="space-y-4 p-6">
      <div className="flex items-center gap-3">
        <Link
          className={buttonVariants({ variant: "outline", size: "icon" })}
          href={props.backHref}
        >
          <ArrowLeftIcon className="size-4 rtl:rotate-180" />
          <span className="sr-only">{t("actions.back")}</span>
        </Link>
        <h4 className="font-bold text-xl">{props.title}</h4>
      </div>
      <Suspense>{props.children}</Suspense>
    </div>
  )
}

function ImportHistories(props: { type: "contacts" | "flow" }) {
  const workspaceId = useWorkspaceId()
  const searchParams = useSearchParams()
  const search = listImportsSearchParamsCache.parse(
    Object.fromEntries(searchParams),
  )
  const input = { ...search, workspaceId, type: importTypes.enum[props.type] }
  const promises = useQueryPromise(
    ["imports", workspaceId, JSON.stringify(input)],
    () => Promise.all([listImports(input)]),
  )
  return <ImportHistoryTable promises={promises} />
}

export function ContactImportPage() {
  const workspaceId = useWorkspaceId()
  return (
    <Gate page="contactsAccess">
      <ImportForm>
        <ImportContactsForm workspaceId={workspaceId} />
      </ImportForm>
    </Gate>
  )
}

export function ContactImportHistoriesPage() {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  return (
    <Gate page="contactsAccess">
      <HistoryPage
        backHref={`/space/${workspaceId}/contacts`}
        title={t("fields.import.histories.title")}
      >
        <ImportHistories type="contacts" />
      </HistoryPage>
    </Gate>
  )
}

export function ContactScanPage() {
  const workspaceId = useWorkspaceId()
  return (
    <Gate page="contactScanAccess">
      <ImportForm>
        <ContactScanForm workspaceId={workspaceId} />
      </ImportForm>
    </Gate>
  )
}

function ContactScanHistories() {
  const workspaceId = useWorkspaceId()
  const searchParams = useSearchParams()
  const search = listContactScanHistorySearchParamsCache.parse(
    Object.fromEntries(searchParams),
  )
  const input = { ...search, workspaceId }
  const promises = useQueryPromise(
    ["contactScanHistory", workspaceId, JSON.stringify(input)],
    () => Promise.all([listContactScanHistory(input)]),
  )
  return <ContactScanHistoryTable promises={promises} />
}

export function ContactScanHistoriesPage() {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  return (
    <Gate page="contactScanAccess">
      <HistoryPage
        backHref={`/space/${workspaceId}/contacts/scan`}
        title={t("contactScan.histories.title")}
      >
        <ContactScanHistories />
      </HistoryPage>
    </Gate>
  )
}

/** `requireWorkspacePermission(workspaceId, "flows")`. */
export function FlowImportHistoriesPage() {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  const { permissions } = useWorkspacePage()
  if (!hasWorkspacePermission(permissions, "flows")) {
    return <NotFoundPage />
  }
  return (
    <HistoryPage
      backHref={`/space/${workspaceId}/flows`}
      title={t("fields.import.histories.title")}
    >
      <ImportHistories type="flow" />
    </HistoryPage>
  )
}
