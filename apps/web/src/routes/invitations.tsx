// page: app/(no-sidebar)/invitations/[code]/page.tsx
import type { InvitationSummary } from "@chatbotx.io/business"
import { useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import { InvitationCard } from "@/features/invitations/invitatation-card"
import { loadPage } from "@/lib/server-action"

export function InvitationsPage() {
  const { code } = useParams({ strict: false }) as { code: string }
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "pages.invitation", code],
    queryFn: () =>
      loadPage<{
        invitation: InvitationSummary | null
        isAuthenticated: boolean
      }>("invitation", { code }),
  })
  return (
    <div className="flex h-screen w-screen items-center justify-center">
      <InvitationCard
        invitation={data.invitation}
        isAuthenticated={data.isAuthenticated}
      />
    </div>
  )
}
