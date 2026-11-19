// page: app/space/[workspaceId]/dynamic-images/page.tsx
// page: app/space/[workspaceId]/dynamic-images/create/page.tsx
// page: app/space/[workspaceId]/dynamic-images/[id]/edit/page.tsx
// page: app/space/[workspaceId]/tools/page.tsx
// page: app/space/[workspaceId]/tools/decision-profiles/page.tsx
// page: app/space/[workspaceId]/leaderboards/page.tsx
// page: app/space/[workspaceId]/leaderboards/create/page.tsx
// page: app/space/[workspaceId]/leaderboards/[id]/edit/page.tsx
// page: app/space/[workspaceId]/leaderboards/[id]/participants/page.tsx
// The Q.D13b tools pages. Membership is the workspace layout's check.
import { Button } from "@chatbotx.io/ui/components/ui/button"
import { Card, CardContent } from "@chatbotx.io/ui/components/ui/card"
import { useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import Link from "next/link"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { type ReactNode, Suspense, useMemo } from "react"
import { AppBreadcrumb } from "@/components/app-breadcrumb"
import { listDecisionConnections } from "@/features/decision-connections/queries/list-decision-connections.query"
import { DecisionProfilesPage as DecisionProfilesView } from "@/features/decision-profiles/components/decision-profiles-page"
import { listDecisionProfiles } from "@/features/decision-profiles/queries/list-decision-profiles.query"
import { listDecisionProfilesSearchParamsCache } from "@/features/decision-profiles/schema/query"
import { CreateDynamicImageForm } from "@/features/dynamic-images/create-dynamic-image-form"
import { DynamicImagesTable } from "@/features/dynamic-images/dynamic-images-table"
import {
  findDynamicImage,
  listDynamicImages,
} from "@/features/dynamic-images/queries"
import { listDynamicImagesSearchParamsCache } from "@/features/dynamic-images/schema/query"
import { UpdateDynamicImageForm } from "@/features/dynamic-images/update-dynamic-image-form"
import { LeaderboardsList } from "@/features/leaderboards/components/list/leaderboards-list"
import { ParticipantsView } from "@/features/leaderboards/components/participants/participants-view"
import { LeaderboardWizard } from "@/features/leaderboards/components/wizard/leaderboard-wizard"
import {
  listLeaderboardsSearchParamsCache,
  listParticipantsSearchParamsCache,
  parsePlatformFilter,
  parseStatusTab,
} from "@/features/leaderboards/lib/search-params"
import {
  findLeaderboard,
  getLeaderboardWizardData,
  listLeaderboardParticipants,
  listLeaderboards,
} from "@/features/leaderboards/queries"
import { BotSimulatorForm } from "@/features/bot-simulator/components/bot-simulator-form"
import { listIntegrationWebchats } from "@/features/integration-webchat/queries"
import { CategoriesTable } from "@/features/places-near-me/components/categories-table"
import { LocationsTable } from "@/features/places-near-me/components/locations-table"
import { PlacesNearMeView } from "@/features/places-near-me/components/places-near-me-view"
import { parsePlacesNearMeTab } from "@/features/places-near-me/lib/tab"
import {
  listPlaceCategories,
  listPlaceLocations,
} from "@/features/places-near-me/queries"
import { listPlaceLocationsSearchParams } from "@/features/places-near-me/schema/query"
import { ToolsList } from "@/features/tools/tools-list"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import { getBrokerOrigin } from "@/lib/oauth-broker"
import { maxPerPage } from "@/lib/shared-request"
import { NotFoundPage } from "@/routes/root"
import { useListPromise } from "@/routes/tools"
import { useWorkspacePage } from "@/routes/workspace"

function useIds() {
  return useParams({ strict: false }) as { workspaceId: string; id?: string }
}

/** Tools › section › title (the builder pages' breadcrumb). */
function ToolSectionPage(props: {
  section: string
  sectionPath?: string
  title?: string
  fallback?: ReactNode
  children: ReactNode
}) {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const items = [
    { label: t("tools.title"), href: `/space/${workspaceId}/tools` },
    {
      label: props.section,
      href: props.title ? `/space/${workspaceId}/${props.sectionPath}` : "",
    },
  ]
  if (props.title) {
    items.push({ label: props.title, href: "" })
  }
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb items={items} />
      <Suspense fallback={props.fallback}>{props.children}</Suspense>
    </div>
  )
}

// ---- dynamic images ----------------------------------------------------------

export function DynamicImagesPage() {
  const t = useTranslations()
  const promises = useListPromise(
    listDynamicImagesSearchParamsCache,
    listDynamicImages,
  )
  return (
    <ToolSectionPage
      fallback={<div>Loading...</div>}
      section={t("dynamicImages.title")}
    >
      <DynamicImagesTable
        promises={promises}
        workspaceId={useIds().workspaceId}
      />
    </ToolSectionPage>
  )
}

export function CreateDynamicImagePage() {
  const t = useTranslations()
  return (
    <ToolSectionPage
      section={t("dynamicImages.title")}
      sectionPath="dynamic-images"
      title={t("actions.create")}
    >
      <CreateDynamicImageForm workspaceId={useIds().workspaceId} />
    </ToolSectionPage>
  )
}

export function EditDynamicImagePage() {
  const t = useTranslations()
  const { workspaceId, id = "" } = useIds()
  const dynamicImage = useSuspenseQuery({
    queryKey: ["spa", "findDynamicImage", workspaceId, id],
    queryFn: () => findDynamicImage({ workspaceId, id }),
  }).data
  if (!dynamicImage) {
    return <NotFoundPage />
  }
  const publicUrl = `${getBrokerOrigin()}/dynamic-images?dynamicImageId=${dynamicImage.id}&userId={{user_id}}`
  return (
    <ToolSectionPage
      section={t("dynamicImages.title")}
      sectionPath="dynamic-images"
      title={t("actions.edit")}
    >
      <UpdateDynamicImageForm
        dynamicImage={dynamicImage}
        publicUrl={publicUrl}
        workspaceId={workspaceId}
      />
    </ToolSectionPage>
  )
}

// ---- tools list + decisions --------------------------------------------------

/** `resolveGuardedWorkspaceId(params, "flows")`; no tool is preview-only. */
export function ToolsPage() {
  const { permissions } = useWorkspacePage()
  if (!hasWorkspacePermission(permissions, "flows")) {
    return <NotFoundPage />
  }
  return <ToolsList canSeePreviewTools={false} permissions={permissions} />
}

/** `requireWorkspacePermission(workspaceId, "superAdmin")`. */
export function DecisionProfilesPage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const { permissions } = useWorkspacePage()
  const profiles = useListPromise(
    listDecisionProfilesSearchParamsCache,
    listDecisionProfiles,
  )
  const promises = useMemo(
    () =>
      Promise.all([
        listDecisionConnections(workspaceId),
        profiles.then(([page]) => page),
      ]),
    [profiles, workspaceId],
  )
  if (!hasWorkspacePermission(permissions, "superAdmin")) {
    return <NotFoundPage />
  }
  return (
    <ToolSectionPage
      fallback={<div>{t("actions.loading")}</div>}
      section={t("decision.profiles")}
    >
      <DecisionProfilesView promises={promises} workspaceId={workspaceId} />
    </ToolSectionPage>
  )
}

// ---- leaderboards ------------------------------------------------------------

function useSearchKey() {
  const searchParams = useSearchParams()
  return [searchParams, searchParams.toString()] as const
}

export function LeaderboardsPage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const [searchParams, key] = useSearchKey()
  const wizard = useSuspenseQuery({
    queryKey: ["spa", "getLeaderboardWizardData", workspaceId],
    queryFn: () => getLeaderboardWizardData(workspaceId),
  }).data
  // biome-ignore lint/correctness/useExhaustiveDependencies: the search string is the key
  const promises = useMemo(() => {
    const search = listLeaderboardsSearchParamsCache.parse(
      Object.fromEntries(searchParams),
    )
    return Promise.all([
      listLeaderboards({
        workspaceId,
        status: parseStatusTab(search.status),
        platform: parsePlatformFilter(search.platform),
        keyword: search.name?.trim() || null,
        page: search.page,
        perPage: search.perPage,
      }),
    ])
  }, [key, workspaceId])
  const minigameNames = Object.fromEntries(
    wizard.minigameOptions.map((minigame) => [minigame.value, minigame.label]),
  )
  return (
    <ToolSectionPage
      fallback={<div>{t("actions.loading")}</div>}
      section={t("leaderboards.title")}
    >
      <LeaderboardsList
        minigameNames={minigameNames}
        promises={promises}
        workspaceId={workspaceId}
      />
    </ToolSectionPage>
  )
}

export function CreateLeaderboardPage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const wizard = useSuspenseQuery({
    queryKey: ["spa", "getLeaderboardWizardData", workspaceId],
    queryFn: () => getLeaderboardWizardData(workspaceId),
  }).data
  return (
    <ToolSectionPage
      section={t("leaderboards.title")}
      sectionPath="leaderboards"
      title={t("actions.create")}
    >
      <LeaderboardWizard
        defaultRules={wizard.defaultRules}
        minigameOptions={wizard.minigameOptions}
        platformChannels={wizard.platformChannels as never}
        workspaceId={workspaceId}
      />
    </ToolSectionPage>
  )
}

function useLeaderboard() {
  const { workspaceId, id = "" } = useIds()
  return useSuspenseQuery({
    queryKey: ["spa", "findLeaderboard", workspaceId, id],
    queryFn: () => findLeaderboard({ workspaceId, id }),
  }).data
}

export function EditLeaderboardPage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const leaderboard = useLeaderboard()
  const wizard = useSuspenseQuery({
    queryKey: ["spa", "getLeaderboardWizardData", workspaceId],
    queryFn: () => getLeaderboardWizardData(workspaceId),
  }).data
  if (!leaderboard) {
    return <NotFoundPage />
  }
  const listHref = `/space/${workspaceId}/leaderboards`
  return (
    <ToolSectionPage
      section={t("leaderboards.title")}
      sectionPath="leaderboards"
      title={leaderboard.name}
    >
      {leaderboard.availableActions.includes("edit") ? (
        <LeaderboardWizard
          defaultRules={wizard.defaultRules}
          leaderboard={leaderboard as never}
          minigameOptions={wizard.minigameOptions}
          platformChannels={wizard.platformChannels as never}
          workspaceId={workspaceId}
        />
      ) : (
        <Card>
          <CardContent className="flex flex-col items-start gap-3">
            <p className="text-sm">{t("leaderboards.wizard.notEditable")}</p>
            <Button
              nativeButton={false}
              render={<Link href={listHref} />}
              size="sm"
              variant="outline"
            >
              {t("actions.back")}
            </Button>
          </CardContent>
        </Card>
      )}
    </ToolSectionPage>
  )
}

export function LeaderboardParticipantsPage() {
  const t = useTranslations()
  const { workspaceId, id = "" } = useIds()
  const leaderboard = useLeaderboard()
  const [searchParams, key] = useSearchKey()
  // biome-ignore lint/correctness/useExhaustiveDependencies: the search string is the key
  const promises = useMemo(() => {
    const search = listParticipantsSearchParamsCache.parse(
      Object.fromEntries(searchParams),
    )
    return Promise.all([
      listLeaderboardParticipants({
        workspaceId,
        leaderboardId: id,
        keyword: search.participant?.trim() || null,
        filter: search.filter,
        page: search.page,
        perPage: search.perPage,
      }),
    ])
  }, [key, workspaceId, id])
  if (!leaderboard) {
    return <NotFoundPage />
  }
  return (
    <ToolSectionPage
      fallback={<div>{t("actions.loading")}</div>}
      section={t("leaderboards.title")}
      sectionPath="leaderboards"
      title={leaderboard.name}
    >
      <ParticipantsView
        leaderboard={leaderboard as never}
        promises={promises}
        workspaceId={workspaceId}
      />
    </ToolSectionPage>
  )
}

// ---- places near me ------------------------------------------------------------

export function PlacesNearMePage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const [searchParams, key] = useSearchKey()
  const tab = parsePlacesNearMeTab(searchParams.get("tab") ?? undefined)
  // Only the active tab's data; locations also need the categories (form).
  const { categories, locations } = useSuspenseQuery({
    queryKey: ["spa", "placesNearMe", workspaceId, key],
    queryFn: async () => {
      const categories = await listPlaceCategories(workspaceId)
      if (tab !== "locations") {
        return { categories, locations: null }
      }
      const search = listPlaceLocationsSearchParams.parse(
        Object.fromEntries(searchParams),
      )
      const locations = await listPlaceLocations({
        workspaceId,
        name: search.name ?? undefined,
        page: Math.max(1, search.page),
        perPage: Math.min(50, Math.max(1, search.perPage)),
      })
      return { categories, locations }
    },
  }).data
  return (
    <ToolSectionPage section={t("placesNearMe.title")}>
      <PlacesNearMeView
        categories={
          <CategoriesTable categories={categories} workspaceId={workspaceId} />
        }
        locations={
          locations ? (
            <LocationsTable
              categories={categories}
              locations={locations}
              workspaceId={workspaceId}
            />
          ) : null
        }
        tab={tab}
      />
    </ToolSectionPage>
  )
}

// ---- bot simulator -------------------------------------------------------------

export function BotSimulatorPage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const webchats = useSuspenseQuery({
    queryKey: ["spa", "listIntegrationWebchats", workspaceId, "simulator"],
    queryFn: () =>
      listIntegrationWebchats({ workspaceId, page: 1, perPage: maxPerPage }),
  }).data.data
  return (
    <ToolSectionPage section={t("botSimulator.title")}>
      <BotSimulatorForm
        webchats={webchats.map((webchat) => ({
          id: webchat.id,
          name: webchat.name,
          enable: webchat.enable,
          authorizedDomains: webchat.authorizedDomains,
        }))}
        workspaceId={workspaceId}
      />
    </ToolSectionPage>
  )
}
