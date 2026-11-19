// page: app/leaderboards/page.tsx
// page: app/minigames/page.tsx
// page: app/bs/[workspaceId]/[webchatId]/page.tsx
// The public leaderboard and minigame pages (lib/public-routes.ts: no
// session). The signed token is the authority: the Rust loaders
// `pages.leaderboardPublic` / `pages.minigamePlay` verify it.
import type { MinigameModel } from "@chatbotx.io/database/types"
import { useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { SimulatorWidget } from "@/features/bot-simulator/components/simulator-widget"
import { LeaderboardPublicView } from "@/features/leaderboards/components/public/leaderboard-public-view"
import type {
  PublicLeaderboardBoard,
  PublicLeaderboardViewer,
} from "@/features/leaderboards/lib/public-board"
import { MINIGAME_PLAY_SCREENS } from "@/features/minigames/components/play/minigame-play-screen-registry"
import { loadPage } from "@/lib/server-action"
import { NotFoundPage } from "@/routes/root"

type LeaderboardPage = {
  board: PublicLeaderboardBoard
  viewer: PublicLeaderboardViewer | null
  token: string | null
}

export function LeaderboardPublicPage() {
  const searchParams = useSearchParams()
  const leaderboardId = searchParams.get("leaderboardId") ?? ""
  const token = searchParams.get("token") ?? undefined
  const page = useSuspenseQuery({
    queryKey: ["page", "leaderboardPublic", leaderboardId, token],
    queryFn: () =>
      loadPage<LeaderboardPage | null>("leaderboardPublic", {
        leaderboardId,
        token,
      }),
  }).data
  if (!page) {
    return <NotFoundPage />
  }
  return (
    <LeaderboardPublicView
      initialBoard={page.board}
      initialViewer={page.viewer}
      token={page.token}
    />
  )
}

type MinigamePlayable = {
  status: "ok"
  minigame: MinigameModel
  contactState: { remaining: number }
  shareUrl: string | null
  token: string
}
type MinigamePage = { status: "comingSoon" | "forbidden" } | MinigamePlayable

function MinigameNotice(props: { title: string; description: string }) {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-2 p-6 text-center">
      <h1 className="font-semibold text-xl">{props.title}</h1>
      <p className="text-muted-foreground">{props.description}</p>
    </div>
  )
}

export function MinigamePlayPage() {
  const t = useTranslations("minigames.play")
  const searchParams = useSearchParams()
  const minigameId = searchParams.get("minigameId") ?? ""
  const token = searchParams.get("token") ?? undefined
  const page = useSuspenseQuery({
    queryKey: ["page", "minigamePlay", minigameId, token],
    queryFn: () =>
      loadPage<MinigamePage | null>("minigamePlay", { minigameId, token }),
  }).data
  if (!page) {
    return <NotFoundPage />
  }
  if (page.status === "comingSoon") {
    return (
      <MinigameNotice
        description={t("comingSoonDescription")}
        title={t("comingSoonTitle")}
      />
    )
  }
  if (page.status === "forbidden") {
    return (
      <MinigameNotice
        description={t("forbiddenDescription")}
        title={t("forbiddenTitle")}
      />
    )
  }
  const game = page as MinigamePlayable
  const PlayScreen = MINIGAME_PLAY_SCREENS[game.minigame.type]
  if (!PlayScreen) {
    return <NotFoundPage />
  }
  return (
    <PlayScreen
      contactState={game.contactState as never}
      minigame={game.minigame}
      shareUrl={game.shareUrl}
      token={game.token}
    />
  )
}

type BotSimulatorPreview = {
  webchatId: string
  brandColor: string
  websiteUrl: string
}

/** `/bs/<workspace>/<webchat>?url=`: the website framed with the webchat on top. */
export function BotSimulatorPreviewPage() {
  const t = useTranslations()
  const { workspaceId, webchatId } = useParams({ strict: false }) as {
    workspaceId: string
    webchatId: string
  }
  const url = useSearchParams().get("url") ?? ""
  const page = useSuspenseQuery({
    queryKey: ["page", "botSimulatorPreview", workspaceId, webchatId, url],
    queryFn: () =>
      loadPage<BotSimulatorPreview | null>("botSimulatorPreview", {
        workspaceId,
        webchatId,
        url,
      }),
  }).data
  if (!page) {
    return <NotFoundPage />
  }
  return (
    <main className="fixed inset-0 bg-background">
      {/* Sandboxed without allow-top-navigation so a frame-busting site
          cannot navigate the simulator away from itself. */}
      <iframe
        className="size-full border-0"
        sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox"
        src={page.websiteUrl}
        title={t("botSimulator.websitePreview")}
      />
      <SimulatorWidget
        brandColor={page.brandColor}
        webchatId={page.webchatId}
        websiteUrl={page.websiteUrl}
        workspaceId={workspaceId}
      />
    </main>
  )
}
