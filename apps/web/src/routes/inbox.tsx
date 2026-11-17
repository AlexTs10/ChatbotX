// page: app/space/[workspaceId]/inbox/page.tsx
// `requireContactsAccess` + `buildContactPermissionScope` are the Rust page
// loader `pages.contactsAccess` (it answers 404 without contacts access).
// ponytail: no server seed (`getInboxInitialState`): the chat store loads the
// first conversations / messages on the client, the builder's fallback path.
import { useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import { FullBleed } from "@/components/full-bleed"
import { ChatLayout } from "@/features/chat/chat-layout"
import { ChatStoreProvider } from "@/features/chat/store/chat-store-provider"
import { loadPage } from "@/lib/server-action"

const DEFAULT_LAYOUT: [number, number, number] = [25, 50, 25]

function savedLayout(): [number, number, number] {
  const raw = document.cookie
    .split("; ")
    .find((part) => part.startsWith("csm:layout:inbox="))
    ?.slice("csm:layout:inbox=".length)
  try {
    return raw ? JSON.parse(decodeURIComponent(raw)) : DEFAULT_LAYOUT
  } catch {
    return DEFAULT_LAYOUT
  }
}

export function InboxPage() {
  const { workspaceId } = useParams({ strict: false }) as {
    workspaceId: string
  }
  const { data: scope } = useSuspenseQuery({
    queryKey: ["spa", "contactsAccess", workspaceId],
    queryFn: () =>
      loadPage<{ canViewEmailAndPhone: boolean }>("contactsAccess", {
        workspaceId,
      }),
  })

  return (
    <FullBleed>
      <ChatStoreProvider>
        <ChatLayout
          canViewEmailAndPhone={scope.canViewEmailAndPhone}
          key={workspaceId}
          layout={savedLayout()}
          workspaceId={workspaceId}
        />
      </ChatStoreProvider>
    </FullBleed>
  )
}
