// apps/builder/src/features/templates/components/locked-editor-guard.tsx for
// the SPA: the lock check is `templatesAPI.listLockedIdsAPI` (the same
// `listLockedTemplateResourceIds` query) instead of a server component.
import type { TemplateResourceKind } from "@chatbotx.io/database/partials"
import { useSuspenseQuery } from "@tanstack/react-query"
import { useTranslations } from "next-intl"
import type { ReactNode } from "react"
import { client } from "@/lib/orpc/orpc"
import { LockedEditorProvider } from "./locked-editor"

type LockedEditorGuardProps = {
  workspaceId: string
  resourceKind: TemplateResourceKind
  resourceId: string
  children: ReactNode
}

/**
 * Wraps an editor page. A copy installed from a View-only template opens with a
 * notice and marks the page locked for the client forms underneath: each form
 * disables only the controls of its content keys (`LockedContent`), so
 * installer-owned controls and Save stay usable. The write paths refuse locked
 * edits on their own (`templateAllowEditViolation`), so this only spares the
 * user a failed save.
 */
export function LockedEditorGuard({
  workspaceId,
  resourceKind,
  resourceId,
  children,
}: LockedEditorGuardProps) {
  const t = useTranslations("templates.locked")
  const input = { workspaceId, resourceKind, resourceIds: [resourceId] }
  const { lockedIds } = useSuspenseQuery({
    queryKey: ["spa", "listLockedIds", input],
    queryFn: () =>
      client.templatesAPI.listLockedIdsAPI(input) as Promise<{
        lockedIds: string[]
      }>,
  }).data
  if (lockedIds.length === 0) {
    return (
      <LockedEditorProvider locked={false}>{children}</LockedEditorProvider>
    )
  }
  return (
    <LockedEditorProvider locked>
      <div
        className="mb-4 rounded-md border bg-muted px-4 py-2 text-muted-foreground text-sm"
        data-testid="template-locked-banner"
        role="status"
      >
        {t("editorBanner")}
      </div>
      {children}
    </LockedEditorProvider>
  )
}
