// page: app/space/[workspaceId]/mini-apps/page.tsx
// page: app/space/[workspaceId]/mini-apps/[id]/layout.tsx
// page: app/space/[workspaceId]/mini-apps/[id]/edit/page.tsx
// page: app/space/[workspaceId]/mini-apps/[id]/submissions/page.tsx
// page: app/mini-apps/page.tsx
// The Mini App pages (Q.D1m). Membership is the workspace layout's check; the
// public runner page (`/mini-apps?id=&token=`) needs no session.
import type { MiniAppDefinition } from "@chatbotx.io/mini-app"
import { useQuery, useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { type ReactNode, Suspense, useMemo } from "react"
import { MiniAppTab } from "@/app/space/[workspaceId]/mini-apps/[id]/tab"
import { AppBreadcrumb } from "@/components/app-breadcrumb"
import { MiniAppEditor } from "@/features/mini-apps/editor/mini-app-editor"
import { buildMiniAppPublicUrl } from "@/features/mini-apps/lib/public-url"
import { MiniAppsTable } from "@/features/mini-apps/mini-apps-table"
import {
  findMiniApp,
  listMiniAppSubmissions,
  listMiniApps,
  listWhatsappPublishTargets,
} from "@/features/mini-apps/queries"
import { PublicMiniApp } from "@/features/mini-apps/runner/public-mini-app"
import {
  listMiniAppSubmissionsSearchParamsCache,
  listMiniAppsSearchParamsCache,
} from "@/features/mini-apps/schema/query"
import { SubmissionsTable } from "@/features/mini-apps/submissions-table"
import { loadPage } from "@/lib/server-action"
import { NotFoundPage } from "@/routes/root"

function useIds() {
  return useParams({ strict: false }) as { workspaceId: string; id?: string }
}

/** The page's promises, made once per search string. */
function useSearchMemo<T>(load: (search: Record<string, string>) => T) {
  const searchParams = useSearchParams()
  const key = searchParams.toString()
  // biome-ignore lint/correctness/useExhaustiveDependencies: the search string is the key
  return useMemo(() => load(Object.fromEntries(searchParams)), [key])
}

export function MiniAppsPage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const promises = useSearchMemo((search) =>
    Promise.all([
      listMiniApps({
        ...listMiniAppsSearchParamsCache.parse(search),
        workspaceId,
      }),
      listWhatsappPublishTargets(workspaceId),
    ]),
  )
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb
        items={[
          { label: t("tools.title"), href: `/space/${workspaceId}/tools` },
          { label: t("miniApps.title"), href: "" },
        ]}
      />
      <Suspense fallback={<div>{t("actions.loading")}</div>}>
        <MiniAppsTable promises={promises} workspaceId={workspaceId} />
      </Suspense>
    </div>
  )
}

type FoundMiniApp = NonNullable<Awaited<ReturnType<typeof findMiniApp>>>

/** `mini-apps/[id]/layout.tsx`: the breadcrumb and the edit / submissions tabs. */
function MiniAppFrame(props: {
  children: (miniApp: FoundMiniApp) => ReactNode
}) {
  const t = useTranslations()
  const { workspaceId, id = "" } = useIds()
  const miniApp = useSuspenseQuery({
    queryKey: ["spa", "findMiniApp", workspaceId, id],
    queryFn: () => findMiniApp({ workspaceId, id }),
  }).data
  if (!miniApp) {
    return <NotFoundPage />
  }
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb
        items={[
          { label: t("tools.title"), href: `/space/${workspaceId}/tools` },
          {
            label: t("miniApps.title"),
            href: `/space/${workspaceId}/mini-apps`,
          },
          { label: miniApp.name, href: "" },
        ]}
      />
      <MiniAppTab miniAppId={id} />
      {props.children(miniApp)}
    </div>
  )
}

export function EditMiniAppPage() {
  const { workspaceId } = useIds()
  const targets = useSuspenseQuery({
    queryKey: ["spa", "listWhatsappPublishTargets", workspaceId],
    queryFn: () => listWhatsappPublishTargets(workspaceId),
  }).data
  return (
    <MiniAppFrame>
      {(miniApp) => (
        <MiniAppEditor
          initialDefinition={miniApp.definition as MiniAppDefinition}
          initialName={miniApp.name}
          // Remount after a save so the editor starts from the stored version.
          key={new Date(miniApp.updatedAt).toISOString()}
          miniAppId={miniApp.id}
          publicUrl={buildMiniAppPublicUrl(miniApp.id)}
          publishTargets={targets}
          workspaceId={workspaceId}
        />
      )}
    </MiniAppFrame>
  )
}

export function MiniAppSubmissionsPage() {
  const { workspaceId, id = "" } = useIds()
  const promises = useSearchMemo((search) =>
    Promise.all([
      listMiniAppSubmissions({
        ...listMiniAppSubmissionsSearchParamsCache.parse(search),
        workspaceId,
        miniAppId: id,
      }),
    ]),
  )
  return (
    <MiniAppFrame>
      {(miniApp) => (
        <Suspense>
          <SubmissionsTable
            definition={miniApp.definition as MiniAppDefinition}
            promises={promises}
            workspaceId={workspaceId}
          />
        </Suspense>
      )}
    </MiniAppFrame>
  )
}

type PublicMiniAppData = {
  miniApp: {
    id: string
    name: string
    definition: MiniAppDefinition
    token: string | null
  } | null
}

/** `app/mini-apps/page.tsx`: the public runner; the server resolves the contact's variables. */
export function PublicMiniAppPage() {
  const t = useTranslations("miniApps.public")
  const searchParams = useSearchParams()
  const id = searchParams.get("id") ?? ""
  const token = searchParams.get("token") ?? undefined
  const resume = searchParams.get("resume") ?? undefined
  const page = useQuery({
    queryKey: ["spa", "publicMiniApp", id, token],
    queryFn: () => loadPage<PublicMiniAppData>("miniApp", { id, token }),
  })
  if (page.isPending) {
    return null
  }
  const miniApp = page.data?.miniApp
  if (!miniApp) {
    return (
      <div className="flex min-h-screen flex-col items-center justify-center gap-2 p-6 text-center">
        <h1 className="font-semibold text-xl">{t("notFoundTitle")}</h1>
        <p className="text-muted-foreground">{t("notFoundDescription")}</p>
      </div>
    )
  }
  return (
    <div className="flex min-h-screen justify-center bg-[#f0f2f5]">
      <div className="flex w-full max-w-md flex-col shadow-sm">
        <PublicMiniApp
          definition={miniApp.definition}
          miniAppId={miniApp.id}
          resume={resume}
          token={miniApp.token ?? undefined}
        />
      </div>
    </div>
  )
}
