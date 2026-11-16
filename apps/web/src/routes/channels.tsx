// page: app/(no-sidebar)/channels/create/page.tsx
// The server part (super-admin check of `workspaceId`, offered / configured
// channels) is the Rust page loader `pages.channelsCreate`.
import type { ChannelType } from "@chatbotx.io/database/partials"
import { useSuspenseQuery } from "@tanstack/react-query"
import { useSearchParams } from "next/navigation"
import InboxSelectCard from "@/features/inboxes/components/inbox-select-card"
import { CreateApiForm } from "@/features/integration-api/components/create-api-form"
import { SimpleCreateWebchat } from "@/features/integration-webchat/simple-create-webchat"
import { loadPage } from "@/lib/server-action"
import {
  CREATE_CHANNEL_ERROR_MESSAGE_KEYS,
  isCreateChannelErrorCode,
} from "@/lib/workspace/create-first-workspace"

type ChannelsCreateData = {
  offeredChannels: ChannelType[]
  configuredChannels: ChannelType[]
}

export function CreateChannelPage() {
  const searchParams = useSearchParams()
  const workspaceId = searchParams.get("workspaceId") || null
  const channel = searchParams.get("channel")
  const error = searchParams.get("error")
  const { data } = useSuspenseQuery({
    queryKey: ["spa", "channelsCreate", workspaceId],
    queryFn: () =>
      loadPage<ChannelsCreateData>("channelsCreate", {
        workspaceId: workspaceId ?? undefined,
      }),
  })
  const isVisible = (type: ChannelType) => data.offeredChannels.includes(type)

  if (channel === "webchat" && isVisible("webchat")) {
    return <SimpleCreateWebchat workspaceId={workspaceId} />
  }
  if (channel === "api" && isVisible("api")) {
    return <CreateApiForm autoOpen={true} workspaceId={workspaceId} />
  }

  return (
    <InboxSelectCard
      configuredChannels={data.configuredChannels}
      errorMessageKey={
        isCreateChannelErrorCode(error)
          ? CREATE_CHANNEL_ERROR_MESSAGE_KEYS[error]
          : undefined
      }
      offeredChannels={data.offeredChannels}
    />
  )
}
