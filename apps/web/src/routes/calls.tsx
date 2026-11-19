// The Calls page (`app/space/[workspaceId]/calls/page.tsx`): the first page,
// the filter options and the admin-only agent filter come from the `callsPage`
// query (Rust); "Load more" calls `listWhatsappCallsAction`. Also the channel
// "Calls" tab (`whatsappCallsPage`).
import { useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import type { WhatsappCallingPreflight } from "@/features/integration-whatsapp/calling/get-whatsapp-calling-preflight"
import type { WhatsappCallingSettings } from "@/features/integration-whatsapp/calling/lib/calling-types"
import { WhatsappCallsCard } from "@/features/integration-whatsapp/calling/whatsapp-calls-card"
import { CallsPageClient } from "@/features/whatsapp-calls/calls-page-client"
import type { CallFilterOption } from "@/features/whatsapp-calls/calls-filter-bar"
import type { CALL_ACTIVITY_CHIPS } from "@/features/whatsapp-calls/schema/query"
import type { WhatsappCallHistoryResource } from "@/features/whatsapp-calls/schema/resource"
import {
  hasContactsAccess,
  hasWorkspacePermission,
} from "@/lib/auth/permission-routes"
import { serverQuery } from "@/lib/server-action"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"

type CallsPageData = {
  data: WhatsappCallHistoryResource[]
  nextCursor: string | null
  inboxOptions: CallFilterOption[]
  agentOptions: CallFilterOption[]
  showAgentFilter: boolean
  activity?: (typeof CALL_ACTIVITY_CHIPS)[number]
  inboxId?: string
  agentUserId?: string
}

const isDigits = (value: string | null) =>
  value && /^\d+$/.test(value) ? value : undefined

export function CallsPage() {
  const { permissions } = useWorkspacePage()
  // Page access: hasContactsAccess || analytics (requireCallHistoryAccess).
  if (
    !(
      hasContactsAccess(permissions) ||
      hasWorkspacePermission(permissions, "analytics")
    )
  ) {
    return <NotFoundPage />
  }
  return <CallsView />
}

function CallsView() {
  const { workspaceId } = useParams({ strict: false }) as {
    workspaceId: string
  }
  const t = useTranslations("whatsapp.calls.page")
  const searchParams = useSearchParams()
  const activity = searchParams.get("activity")
  const input = {
    activity:
      activity === "missed" || activity === "noReply" ? activity : undefined,
    // A non-numeric id is "no filter" (parseAsBigInt).
    inboxId: isDigits(searchParams.get("inboxId")),
    agentUserId: isDigits(searchParams.get("agentUserId")),
  }
  const page = useSuspenseQuery({
    queryKey: ["spa", "callsPage", workspaceId, JSON.stringify(input)],
    queryFn: () =>
      serverQuery<CallsPageData>("callsPage", workspaceId, [
        workspaceId,
        input,
      ]),
  }).data
  return (
    <div className="flex flex-col gap-4">
      <div>
        <h1 className="font-semibold text-xl">{t("title")}</h1>
        <p className="text-muted-foreground text-sm">{t("subtitle")}</p>
      </div>
      <CallsPageClient
        activity={page.activity}
        agentOptions={page.agentOptions}
        agentUserId={page.agentUserId}
        inboxId={page.inboxId}
        inboxOptions={page.inboxOptions}
        initialData={page.data}
        initialNextCursor={page.nextCursor}
        key={`${page.activity ?? "all"}:${page.inboxId ?? "all"}:${page.agentUserId ?? "all"}`}
        showAgentFilter={page.showAgentFilter}
        workspaceId={workspaceId}
      />
    </div>
  )
}

type WhatsappCallsSettingsData = {
  inboundCallsEnabled: boolean
  recordingEnabled: boolean
  recordingRetentionDays: number
  transcriptionEnabled: boolean
  settings: WhatsappCallingSettings | null
  loadError: string | null
  preflight: WhatsappCallingPreflight
  isSuperAdmin: boolean
  workspaceTimezone: string
}

/** The channel "Calls" tab (`(integrations)/whatsapps/[id]/calls/page.tsx`). */
export function WhatsappCallsSettingsPage() {
  const { workspaceId, id } = useParams({ strict: false }) as {
    workspaceId: string
    id: string
  }
  const { permissions } = useWorkspacePage()
  // The `whatsapps/[id]` layout guard: super admins only.
  if (!hasWorkspacePermission(permissions, "superAdmin") || !/^\d+$/.test(id)) {
    return <NotFoundPage />
  }
  return <WhatsappCallsSettingsView id={id} workspaceId={workspaceId} />
}

function WhatsappCallsSettingsView({
  workspaceId,
  id,
}: {
  workspaceId: string
  id: string
}) {
  const page = useSuspenseQuery({
    queryKey: ["spa", "whatsappCallsPage", workspaceId, id],
    queryFn: () =>
      serverQuery<WhatsappCallsSettingsData>("whatsappCallsPage", workspaceId, [
        workspaceId,
        { id },
      ]),
  }).data
  return (
    <WhatsappCallsCard
      inboundCallsEnabled={page.inboundCallsEnabled}
      integrationWhatsappId={id}
      isSuperAdmin={page.isSuperAdmin}
      loadError={page.loadError ?? undefined}
      preflight={page.preflight}
      recordingEnabled={page.recordingEnabled}
      recordingRetentionDays={page.recordingRetentionDays}
      settings={page.settings}
      transcriptionEnabled={page.transcriptionEnabled}
      workspaceId={workspaceId}
      workspaceTimezone={page.workspaceTimezone}
    />
  )
}
