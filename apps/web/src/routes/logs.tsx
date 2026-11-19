// page: app/space/[workspaceId]/error-logs/page.tsx
// page: app/space/[workspaceId]/(enterprise)/layout.tsx
// page: app/space/[workspaceId]/(enterprise)/audit-logs/page.tsx
// page: app/space/[workspaceId]/(enterprise)/pricing/page.tsx
import { PricingTableFour } from "@chatbotx.io/ui/components/billingsdk/pricing-table-four"
import { samplePlans } from "@chatbotx.io/ui/lib/billingsdk-config"
import { useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { Suspense, useMemo } from "react"
import { AppTab } from "@/components/app-tab"
import { AuditLogsTable } from "@/enterprise/features/audit-logs/audit-logs-table"
import {
  listAuditLogAdmins,
  listAuditLogs,
} from "@/enterprise/features/audit-logs/queries"
import { listAuditLogsSearchParamsCache } from "@/enterprise/features/audit-logs/schema/query"
import { isCommunity } from "@/env"
import { ErrorLogsTable } from "@/features/error-logs/error-logs-table"
import { listErrorLogs } from "@/features/error-logs/queries"
import { listErrorLogsSearchParamsCache } from "@/features/error-logs/schema/query"
import { NotFoundPage } from "@/routes/root"

function useWorkspaceId() {
  return (useParams({ strict: false }) as { workspaceId: string }).workspaceId
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

export function ErrorLogsPage() {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  const search = useSearch(listErrorLogsSearchParamsCache)
  const promises = useMemo(
    () => Promise.all([listErrorLogs({ ...search, workspaceId })]),
    [search, workspaceId],
  )
  const tab = (label: string, segment: string) => ({
    label,
    href: `/space/${workspaceId}/${segment}`,
    isActive: segment === "error-logs",
  })

  return (
    <div className="flex flex-col gap-4">
      <AppTab
        tabs={[
          tab(t("flows.title"), "flows"),
          tab(t("tags.title"), "tags"),
          tab(t("customFields.title"), "custom-fields"),
          tab(t("emailTopics.title"), "email-topics"),
          tab(t("errorLogs.title"), "error-logs"),
        ]}
      />
      <Suspense>
        <ErrorLogsTable promises={promises} workspaceId={workspaceId} />
      </Suspense>
    </div>
  )
}

export function AuditLogsPage() {
  // `(enterprise)/layout.tsx`: no enterprise pages in the community edition.
  return isCommunity() ? <NotFoundPage /> : <AuditLogs />
}

function AuditLogs() {
  const workspaceId = useWorkspaceId()
  const search = useSearch(listAuditLogsSearchParamsCache)
  const promises = useMemo(
    () =>
      Promise.all([
        listAuditLogs({ ...search, workspaceId }),
        listAuditLogAdmins(workspaceId),
      ]),
    [search, workspaceId],
  )
  return (
    <div>
      <Suspense>
        <AuditLogsTable promises={promises} search={search} />
      </Suspense>
    </div>
  )
}

export function PricingPage() {
  return isCommunity() ? <NotFoundPage /> : <Pricing />
}

// ponytail: the original page shows the sample plans and only logs the
// selection (its upgrade call is commented out); kept as is.
function Pricing() {
  return (
    <PricingTableFour
      billingToggleLabels={{ monthly: "Monthly", yearly: "Yearly" }}
      className="w-full"
      description="Transform your project with our comprehensive pricing options designed for every need."
      onPlanSelect={(planId: string) => console.log("Selected plan:", planId)}
      plans={samplePlans}
      showBillingToggle={false}
      size="medium"
      subtitle="Simple Pricing"
      theme="classic"
      title="Choose Your Perfect Plan"
    />
  )
}
