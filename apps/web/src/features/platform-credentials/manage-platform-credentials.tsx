// The SPA form of apps/builder/src/features/platform-credentials/manage-platform-credentials.tsx:
// the Rust server resolves the cards (`getPlatformCredentialCards`: the
// scope checks, `resolveCard`, the origins and the webhook URLs).
import type {
  CredentialPublicByType,
  CredentialType,
} from "@chatbotx.io/database/partials"
import { ORPCError } from "@orpc/client"
import { useSuspenseQuery } from "@tanstack/react-query"
import { serverFunction } from "@/lib/server-action"
import { NotFoundPage } from "@/routes/root"
import { GiphySettings } from "./giphy/giphy-settings"
import { GoogleSettings } from "./google/google-settings"
import { GoogleAdsSettings } from "./google-ads/google-ads-settings"
import { InstagramSettings } from "./instagram/instagram-settings"
import { InstagramFacebookSettings } from "./instagram-facebook/instagram-facebook-settings"
import { MakeSettings } from "./make/make-settings"
import { MessengerSettings } from "./messenger/messenger-settings"
import { CredentialScopeProvider } from "./provider/credential-scope-context"
import type { CredentialScope } from "./scope"
import { StripeSettings } from "./stripe/stripe-settings"
import { ThreadsSettings } from "./threads/threads-settings"
import { TiktokSettings } from "./tiktok/tiktok-settings"
import { WhatsappSettings } from "./whatsapp/whatsapp-settings"
import { ZaloSettings } from "./zalo/zalo-settings"

type Card<T extends CredentialType> = {
  credentialId: string | null
  publicConfig: CredentialPublicByType[T] | null
  isInherited: boolean
  callbackOrigin: string
  webhookUrl: string
}

type CardTypes =
  | "whatsapp"
  | "messenger"
  | "instagram"
  | "instagramFacebook"
  | "threads"
  | "google"
  | "googleAds"
  | "zalo"
  | "giphy"
  | "tiktok"
  | "make"
  | "stripe"

type Cards = { [T in CardTypes]: Card<T> } & { hasStripeWebhook: boolean }

const getPlatformCredentialCards = serverFunction("getPlatformCredentialCards")

export function ManagePlatformCredentials({
  scope = "user",
}: {
  scope?: CredentialScope
}) {
  const { data: cards } = useSuspenseQuery({
    queryKey: ["spa", "getPlatformCredentialCards", scope],
    queryFn: async (): Promise<Cards | null> => {
      try {
        return await getPlatformCredentialCards(scope)
      } catch (error) {
        if (error instanceof ORPCError && error.status === 404) {
          return null
        }
        throw error
      }
    },
  })
  if (!cards) {
    return <NotFoundPage />
  }
  const card = <T extends CardTypes>(
    type: T,
  ): Pick<Card<T>, "callbackOrigin" | "isInherited" | "publicConfig"> => {
    const { callbackOrigin, isInherited, publicConfig } = cards[
      type
    ] as Card<T>
    return { callbackOrigin, isInherited, publicConfig }
  }
  const webhookUrl = (type: CardTypes) => cards[type].webhookUrl

  return (
    <CredentialScopeProvider scope={scope}>
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <MessengerSettings
          {...card("messenger")}
          webhookUrl={webhookUrl("messenger")}
        />
        <InstagramSettings
          {...card("instagram")}
          webhookUrl={webhookUrl("instagram")}
        />
        <InstagramFacebookSettings
          {...card("instagramFacebook")}
          webhookUrl={webhookUrl("instagramFacebook")}
        />
        <ThreadsSettings {...card("threads")} />
        <GoogleSettings {...card("google")} />
        <GoogleAdsSettings {...card("googleAds")} />
        <WhatsappSettings
          {...card("whatsapp")}
          webhookUrl={webhookUrl("whatsapp")}
        />
        <ZaloSettings {...card("zalo")} webhookUrl={webhookUrl("zalo")} />
        <TiktokSettings
          {...card("tiktok")}
          webhookUrl={webhookUrl("tiktok")}
        />
        <GiphySettings
          isConfigured={cards.giphy.publicConfig !== null}
          isInherited={cards.giphy.isInherited}
        />
        <MakeSettings
          isInherited={cards.make.isInherited}
          publicConfig={cards.make.publicConfig}
        />
        <StripeSettings
          {...card("stripe")}
          hasConnectWebhook={cards.hasStripeWebhook}
        />
      </div>
    </CredentialScopeProvider>
  )
}
