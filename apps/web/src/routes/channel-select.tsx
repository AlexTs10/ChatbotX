// page: app/(no-sidebar)/channels/messenger/select/page.tsx
// page: app/(no-sidebar)/channels/instagram/select/page.tsx
// page: app/(no-sidebar)/channels/instagram-facebook/select/page.tsx
// The account pickers after the provider dialog. `session.targets` is the
// list the Rust connect engine built at authorization time; the loader
// (`pages.channelConnectSelect`, `resolveSelectSession`) checks the session
// belongs to the signed-in user and answers `{ redirect }` otherwise.
import type { ChannelType } from "@chatbotx.io/database/partials"
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@chatbotx.io/ui/components/ui/card"
import { useSuspenseQuery } from "@tanstack/react-query"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { CONNECT_PICKER_CARD_CLASS } from "@/features/channel-connect/components/connect-picker-card"
import type { ConnectPickerItem } from "@/features/channel-connect/lib/picker-items"
import { InboxIcon } from "@/features/inboxes/components/inbox-icon"
import { SelectAccount } from "@/features/integration-instagram/components/select-accounts"
import { SelectFacebookAccounts } from "@/features/integration-instagram/components/select-facebook-accounts"
import { SelectPage } from "@/features/integration-messenger/components/select-account"
import { loadPage } from "@/lib/server-action"
import { Redirect } from "@/shims/redirect"

type Target = {
  id: string
  name: string
  avatarUrl?: string
  selectable: boolean
  alreadyConnected?: "this_workspace" | "other_workspace"
}

type SelectData =
  | { redirect: string }
  | { sessionId: string; workspaceId: string; targets: Target[] }

function useSelectSession(provider: string) {
  const searchParams = useSearchParams()
  const session = searchParams.get("session") ?? ""
  const connectError = searchParams.get("connect_error") ?? undefined
  return useSuspenseQuery({
    queryKey: ["spa", "channelConnectSelect", provider, session, connectError],
    queryFn: () =>
      loadPage<SelectData>("channelConnectSelect", {
        provider,
        session,
        connectError,
      }),
  }).data
}

/** `toConnectPickerItem`. */
function toPickerItem(
  target: Target,
  channel: ChannelType,
  alreadyConnectedLabel: string,
): ConnectPickerItem {
  return {
    id: target.id,
    name: target.name,
    secondary: target.id,
    disabled: !target.selectable,
    disabledReason: target.alreadyConnected ? alreadyConnectedLabel : undefined,
    leading: target.avatarUrl ? (
      <img
        alt={target.name}
        className="size-6 rounded-full object-cover"
        height={24}
        src={target.avatarUrl}
        width={24}
      />
    ) : (
      <InboxIcon channel={channel} showLabel={false} size="small" />
    ),
  }
}

function pickerItems(
  targets: Target[],
  channel: ChannelType,
  alreadyConnectedLabel: string,
) {
  return targets
    .map((target) => toPickerItem(target, channel, alreadyConnectedLabel))
    .sort((current, next) => Number(current.disabled) - Number(next.disabled))
}

export function MessengerSelectPage() {
  const t = useTranslations()
  const data = useSelectSession("messenger")
  if ("redirect" in data) {
    return <Redirect href={data.redirect} />
  }
  return (
    <SelectPage
      items={pickerItems(
        data.targets,
        "messenger",
        t("messenger.selectPage.alreadyConnectedNote"),
      )}
      sessionId={data.sessionId}
      workspaceId={data.workspaceId}
    />
  )
}

export function InstagramSelectPage() {
  const data = useSelectSession("instagram")
  if ("redirect" in data) {
    return <Redirect href={data.redirect} />
  }
  const target = data.targets[0]
  if (!target) {
    return <Redirect href={`/channels/create?workspaceId=${data.workspaceId}`} />
  }
  return (
    <SelectAccount
      account={{
        id: target.id,
        name: target.name,
        avatarUrl: target.avatarUrl,
      }}
      sessionId={data.sessionId}
      workspaceId={data.workspaceId}
    />
  )
}

export function InstagramFacebookSelectPage() {
  const t = useTranslations()
  const data = useSelectSession("instagramFacebook")
  if ("redirect" in data) {
    return <Redirect href={data.redirect} />
  }
  return (
    <Card className={CONNECT_PICKER_CARD_CLASS}>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle>{t("fields.instagram.connectViaFacebook")}</CardTitle>
      </CardHeader>
      <CardContent>
        <SelectFacebookAccounts
          items={pickerItems(
            data.targets,
            "instagram",
            t("instagram.selectPage.alreadyConnectedNote"),
          )}
          sessionId={data.sessionId}
          workspaceId={data.workspaceId}
        />
      </CardContent>
    </Card>
  )
}
