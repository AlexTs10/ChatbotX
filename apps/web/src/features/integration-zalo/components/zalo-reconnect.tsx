"use client"

import { Button } from "@chatbotx.io/ui/components/ui/button"
import { Loader2Icon } from "lucide-react"
import { useTranslations } from "next-intl"
import { useAction } from "next-safe-action/hooks"
import { toast } from "sonner"
import { useWorkspaceId } from "@/hooks/routing"
import { reconnectZaloAction } from "../actions/reconnect.action"
import type { IntegrationZaloResource } from "../queries"

export function ZaloReconnect({
  integrationZalo,
}: {
  integrationZalo: IntegrationZaloResource
}) {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()

  // The action answers the Zalo OAuth dialog URL (signed connect session).
  const { execute, isPending } = useAction(
    reconnectZaloAction.bind(null, workspaceId, integrationZalo.id),
    {
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
      {t("zalo.reconnect")}
    </Button>
  )
}
