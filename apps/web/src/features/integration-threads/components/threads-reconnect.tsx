"use client"

import { Button } from "@chatbotx.io/ui/components/ui/button"
import { Loader2Icon } from "lucide-react"
import { useTranslations } from "next-intl"
import { useAction } from "next-safe-action/hooks"
import { toast } from "sonner"
import { useWorkspaceId } from "@/hooks/routing"
import { reconnectThreadsAction } from "../actions/reconnect.action"
import type { IntegrationThreadsResource } from "../queries"

export function ThreadsReconnect({
  integrationThreads,
}: {
  integrationThreads: IntegrationThreadsResource
}) {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  const { execute, isPending } = useAction(
    reconnectThreadsAction.bind(null, workspaceId, integrationThreads.id),
    {
      // The action answers the Threads OAuth dialog URL (signed connect session).
      onSuccess: ({ data }) => {
        if (data?.url) {
          window.location.assign(data.url)
        }
      },
      onError: ({ error }) => {
        if (error.serverError) {
          toast.error(error.serverError)
        }
      },
    },
  )

  return (
    <Button
      disabled={isPending}
      onClick={() => execute()}
      size="sm"
      variant="secondary"
    >
      {isPending && <Loader2Icon className="animate-spin" />}
      {t("instagram.reconnect")}
    </Button>
  )
}
