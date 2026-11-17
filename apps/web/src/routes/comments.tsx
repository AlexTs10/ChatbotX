// page: app/space/[workspaceId]/(has-folder)/fb-comments/layout.tsx
// page: app/space/[workspaceId]/(has-folder)/fb-comments/page.tsx
// page: app/space/[workspaceId]/(has-folder)/ig-comments/layout.tsx
// page: app/space/[workspaceId]/(has-folder)/ig-comments/page.tsx
// page: app/space/[workspaceId]/(has-folder)/ig-stories/page.tsx
// page: app/space/[workspaceId]/fb-comments/create/page.tsx
// page: app/space/[workspaceId]/fb-comments/[id]/page.tsx
// page: app/space/[workspaceId]/fb-comments/[id]/analytics/page.tsx
// page: app/space/[workspaceId]/ig-comments/create/page.tsx
// page: app/space/[workspaceId]/ig-comments/[id]/page.tsx
// page: app/space/[workspaceId]/ig-comments/[id]/analytics/page.tsx
// page: app/space/[workspaceId]/ig-stories/create/page.tsx
// page: app/space/[workspaceId]/ig-stories/[id]/page.tsx
// page: app/space/[workspaceId]/threads-comments/page.tsx
// page: app/space/[workspaceId]/threads-comments/create/page.tsx
// page: app/space/[workspaceId]/threads-comments/[id]/page.tsx
// page: app/space/[workspaceId]/tiktok-comments/page.tsx
// page: app/space/[workspaceId]/tiktok-comments/create/page.tsx
// page: app/space/[workspaceId]/tiktok-comments/[id]/page.tsx
// page: app/space/[workspaceId]/tiktok-comments/[id]/analytics/page.tsx
// The comment and story automation screens. Membership is the workspace
// layout's check; data comes from the Rust `/rpc` (comment queries/actions).
import { type FolderType, folderTypes } from "@chatbotx.io/database/partials"
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@chatbotx.io/ui/components/ui/card"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Navigate, Outlet, useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { useTranslations } from "next-intl"
import { type ReactNode, Suspense, useMemo } from "react"
import { CreateFbCommentForm } from "@/features/fb-comments/components/create-fb-comment-form"
import { EditFbCommentForm } from "@/features/fb-comments/components/edit-fb-comment-form"
import { FbCommentsTable } from "@/features/fb-comments/fb-comments-table"
import { FbCommentPostsStoreProvider } from "@/features/fb-comments/provider/fb-comment-posts-store-context"
import { getFbComment, listFbComments } from "@/features/fb-comments/queries"
import { listFbCommentsSearchParamsCache } from "@/features/fb-comments/schema/action"
import { FolderStoreProvider } from "@/features/folders/provider/folder-store-context"
import SharedFolderSlot from "@/features/folders/shared-folder-slot"
import { CreateIgCommentForm } from "@/features/ig-comments/components/create-ig-comment-form"
import { EditIgCommentForm } from "@/features/ig-comments/components/edit-ig-comment-form"
import { IgCommentsTable } from "@/features/ig-comments/ig-comments-table"
import { IgCommentPostsStoreProvider } from "@/features/ig-comments/provider/ig-comment-posts-store-context"
import { getIgComment, listIgComments } from "@/features/ig-comments/queries"
import {
  type IgCommentVariant,
  igCommentVariants,
  listIgCommentsSearchParamsCache,
} from "@/features/ig-comments/schema/action"
import { CreateIgStoryForm } from "@/features/ig-stories/components/create-ig-story-form"
import { EditIgStoryForm } from "@/features/ig-stories/components/edit-ig-story-form"
import { IgStoriesTable } from "@/features/ig-stories/ig-stories-table"
import { IgStoryPostsStoreProvider } from "@/features/ig-stories/provider/ig-story-posts-store-context"
import { getIgStory, listIgStories } from "@/features/ig-stories/queries"
import {
  type IgStoryVariant,
  igStoryVariants,
  listIgStoriesSearchParamsCache,
  resolveTriggerFilter,
} from "@/features/ig-stories/schema/action"
import { CommentAutomationAnalyticsClient } from "@/features/shared/comment-automation/comment-automation-analytics-client"
import { isLivePostTypeParam } from "@/features/shared/comment-automation/lib/live-post-type"
import { LockedEditorGuard } from "@/features/templates/components/locked-editor-guard"
import { CreateThreadsCommentForm } from "@/features/threads-comments/components/create-threads-comment-form"
import { EditThreadsCommentForm } from "@/features/threads-comments/components/edit-threads-comment-form"
import {
  getThreadsComment,
  listThreadsComments,
} from "@/features/threads-comments/queries"
import { listThreadsCommentsSearchParamsCache } from "@/features/threads-comments/schema/action"
import { ThreadsCommentsTable } from "@/features/threads-comments/threads-comments-table"
import { CreateTiktokCommentForm } from "@/features/tiktok-comments/components/create-tiktok-comment-form"
import { EditTiktokCommentForm } from "@/features/tiktok-comments/components/edit-tiktok-comment-form"
import {
  getTiktokComment,
  listTiktokComments,
} from "@/features/tiktok-comments/queries"
import { listTiktokCommentsSearchParamsCache } from "@/features/tiktok-comments/schema/action"
import { TiktokCommentsTable } from "@/features/tiktok-comments/tiktok-comments-table"
import { NotFoundPage } from "@/routes/root"

function useIds() {
  return useParams({ strict: false }) as { workspaceId: string; id?: string }
}

function useSearch() {
  const searchParams = useSearchParams()
  return {
    key: searchParams.toString(),
    params: Object.fromEntries(searchParams) as Record<string, string>,
  }
}

/** The list promise of a page, made once per search string. */
function useListPromise<T>(
  parse: (params: Record<string, string>) => unknown,
  load: (input: never) => Promise<T>,
  extra?: (search: Record<string, unknown>) => Record<string, unknown>,
) {
  const { workspaceId } = useIds()
  const { key, params } = useSearch()
  // biome-ignore lint/correctness/useExhaustiveDependencies: the search string is the key
  return useMemo(() => {
    const search = parse(params) as Record<string, unknown>
    const input = { ...search, ...extra?.(search), workspaceId }
    return Promise.all([load(input as never)]) as Promise<[T]>
  }, [key, workspaceId])
}

/** A record of the page (`null` = not found). */
function useRecord<T>(name: string, get: (ws: string, id: string) => Promise<T>) {
  const { workspaceId, id = "" } = useIds()
  return useSuspenseQuery({
    queryKey: ["spa", name, workspaceId, id],
    queryFn: () => get(workspaceId, id).catch(() => null),
  }).data
}

function FolderLayout(props: { folderType: FolderType }) {
  const { workspaceId } = useIds()
  return (
    <FolderStoreProvider
      autoInitialize={true}
      folderType={props.folderType}
      workspaceId={workspaceId}
    >
      <Suspense>
        <Outlet />
      </Suspense>
    </FolderStoreProvider>
  )
}

function ListCard(props: {
  title: string
  folderType: FolderType
  children: ReactNode
}) {
  const { workspaceId } = useIds()
  return (
    <Card>
      <CardHeader>
        <CardTitle className="font-bold text-xl">{props.title}</CardTitle>
      </CardHeader>
      <CardContent>
        <SharedFolderSlot
          folderType={props.folderType}
          workspaceId={workspaceId}
        />
        <Suspense>{props.children}</Suspense>
      </CardContent>
    </Card>
  )
}

function Analytics(props: { name: string }) {
  const { workspaceId, id = "" } = useIds()
  return (
    <div className="container mx-auto flex flex-col gap-6 py-6">
      <CommentAutomationAnalyticsClient
        automationId={id}
        automationName={props.name}
        workspaceId={workspaceId}
      />
    </div>
  )
}

function Locked(props: { children: ReactNode }) {
  const { workspaceId, id = "" } = useIds()
  return (
    <LockedEditorGuard
      resourceId={id}
      resourceKind="fbCommentAutomation"
      workspaceId={workspaceId}
    >
      {props.children}
    </LockedEditorGuard>
  )
}

// ---- Facebook ---------------------------------------------------------------

export function FbCommentsLayout() {
  return <FolderLayout folderType={folderTypes.enum.fbComment} />
}

export function FbCommentsPage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const promises = useListPromise(
    (p) => listFbCommentsSearchParamsCache.parse(p),
    listFbComments,
  )
  return (
    <ListCard
      folderType={folderTypes.enum.fbComment}
      title={t("facebookCommentAutomation.title")}
    >
      <FbCommentsTable promises={promises} workspaceId={workspaceId} />
    </ListCard>
  )
}

export function CreateFbCommentPage() {
  const { workspaceId } = useIds()
  const isLive = isLivePostTypeParam(useSearch().params.postType)
  return (
    <FbCommentPostsStoreProvider
      autoInitialize={!isLive}
      workspaceId={workspaceId}
    >
      <CreateFbCommentForm isLive={isLive} workspaceId={workspaceId} />
    </FbCommentPostsStoreProvider>
  )
}

/** `getFbComment`, else the Instagram automation of a pre-split URL. */
function useFbOrIg() {
  const { workspaceId, id = "" } = useIds()
  return useSuspenseQuery({
    queryKey: ["spa", "getFbComment", workspaceId, id],
    queryFn: async () => {
      const fb = await getFbComment(workspaceId, id).catch(() => null)
      if (fb) {
        return { fb, ig: false }
      }
      const ig = await getIgComment(workspaceId, id).catch(() => null)
      return { fb: null, ig: Boolean(ig) }
    },
  }).data
}

export function EditFbCommentPage() {
  const { workspaceId, id = "" } = useIds()
  const { fb, ig } = useFbOrIg()
  if (!fb) {
    return ig ? (
      <Navigate to={`/space/${workspaceId}/ig-comments/${id}`} />
    ) : (
      <NotFoundPage />
    )
  }
  return (
    <FbCommentPostsStoreProvider autoInitialize={true} workspaceId={workspaceId}>
      <Locked>
        <EditFbCommentForm initialData={fb} workspaceId={workspaceId} />
      </Locked>
    </FbCommentPostsStoreProvider>
  )
}

export function FbCommentAnalyticsPage() {
  const record = useRecord("getFbComment", getFbComment)
  return record ? <Analytics name={record.name} /> : <NotFoundPage />
}

// ---- Instagram comments -----------------------------------------------------

export function IgCommentsLayout() {
  return <FolderLayout folderType={folderTypes.enum.igComment} />
}

export function IgCommentsPage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const promises = useListPromise(
    (p) => listIgCommentsSearchParamsCache.parse(p),
    listIgComments,
  )
  return (
    <ListCard
      folderType={folderTypes.enum.igComment}
      title={t("instagramCommentAutomation.title")}
    >
      <IgCommentsTable promises={promises} workspaceId={workspaceId} />
    </ListCard>
  )
}

export function CreateIgCommentPage() {
  const { workspaceId } = useIds()
  const { params } = useSearch()
  const isLive = isLivePostTypeParam(params.postType)
  const { data: variant } = igCommentVariants.safeParse(params.variant)
  if (!variant) {
    return <Navigate to={`/space/${workspaceId}/ig-comments`} />
  }
  return (
    <IgCommentPostsStoreProvider
      autoInitialize={!isLive}
      variant={variant}
      workspaceId={workspaceId}
    >
      <CreateIgCommentForm
        isLive={isLive}
        variant={variant}
        workspaceId={workspaceId}
      />
    </IgCommentPostsStoreProvider>
  )
}

export function EditIgCommentPage() {
  const { workspaceId } = useIds()
  const record = useRecord("getIgComment", getIgComment)
  if (!record) {
    return <NotFoundPage />
  }
  return (
    <IgCommentPostsStoreProvider
      autoInitialize={true}
      variant={record.type as IgCommentVariant}
      workspaceId={workspaceId}
    >
      <Locked>
        <EditIgCommentForm initialData={record} workspaceId={workspaceId} />
      </Locked>
    </IgCommentPostsStoreProvider>
  )
}

export function IgCommentAnalyticsPage() {
  const record = useRecord("getIgComment", getIgComment)
  return record ? <Analytics name={record.name} /> : <NotFoundPage />
}

// ---- Instagram stories ------------------------------------------------------

export function IgStoriesLayout() {
  return <FolderLayout folderType={folderTypes.enum.igStory} />
}

export function IgStoriesPage() {
  const t = useTranslations()
  const { workspaceId } = useIds()
  const promises = useListPromise(
    (p) => listIgStoriesSearchParamsCache.parse(p),
    listIgStories,
    (search) => ({
      trigger: resolveTriggerFilter(
        search.trigger as Parameters<typeof resolveTriggerFilter>[0],
      ),
    }),
  )
  return (
    <ListCard
      folderType={folderTypes.enum.igStory}
      title={t("instagramStoryAutomation.title")}
    >
      <IgStoriesTable promises={promises} workspaceId={workspaceId} />
    </ListCard>
  )
}

export function CreateIgStoryPage() {
  const { workspaceId } = useIds()
  const { data: variant } = igStoryVariants.safeParse(
    useSearch().params.variant,
  )
  if (!variant) {
    return <Navigate to={`/space/${workspaceId}/ig-stories`} />
  }
  return (
    <IgStoryPostsStoreProvider
      autoInitialize={true}
      variant={variant}
      workspaceId={workspaceId}
    >
      <CreateIgStoryForm variant={variant} workspaceId={workspaceId} />
    </IgStoryPostsStoreProvider>
  )
}

export function EditIgStoryPage() {
  const { workspaceId } = useIds()
  const record = useRecord("getIgStory", getIgStory)
  if (!record) {
    return <NotFoundPage />
  }
  return (
    <IgStoryPostsStoreProvider
      autoInitialize={true}
      variant={record.type as IgStoryVariant}
      workspaceId={workspaceId}
    >
      <EditIgStoryForm initialData={record} workspaceId={workspaceId} />
    </IgStoryPostsStoreProvider>
  )
}

// ---- Threads ----------------------------------------------------------------

export function ThreadsCommentsPage() {
  const { workspaceId } = useIds()
  const promises = useListPromise(
    (p) => listThreadsCommentsSearchParamsCache.parse(p),
    listThreadsComments,
  )
  return (
    <Suspense>
      <ThreadsCommentsTable promises={promises} workspaceId={workspaceId} />
    </Suspense>
  )
}

export function CreateThreadsCommentPage() {
  return <CreateThreadsCommentForm workspaceId={useIds().workspaceId} />
}

export function EditThreadsCommentPage() {
  const { workspaceId } = useIds()
  const record = useRecord("getThreadsComment", getThreadsComment)
  if (!record) {
    return <NotFoundPage />
  }
  return (
    <Locked>
      <EditThreadsCommentForm initialData={record} workspaceId={workspaceId} />
    </Locked>
  )
}

// ---- TikTok -----------------------------------------------------------------

export function TiktokCommentsPage() {
  const { workspaceId } = useIds()
  const promises = useListPromise(
    (p) => listTiktokCommentsSearchParamsCache.parse(p),
    listTiktokComments,
  )
  return (
    <Suspense>
      <TiktokCommentsTable promises={promises} workspaceId={workspaceId} />
    </Suspense>
  )
}

export function CreateTiktokCommentPage() {
  return <CreateTiktokCommentForm workspaceId={useIds().workspaceId} />
}

export function EditTiktokCommentPage() {
  const { workspaceId } = useIds()
  const record = useRecord("getTiktokComment", getTiktokComment)
  if (!record) {
    return <NotFoundPage />
  }
  return (
    <Locked>
      <EditTiktokCommentForm initialData={record} workspaceId={workspaceId} />
    </Locked>
  )
}

export function TiktokCommentAnalyticsPage() {
  const record = useRecord("getTiktokComment", getTiktokComment)
  return record ? <Analytics name={record.name} /> : <NotFoundPage />
}
