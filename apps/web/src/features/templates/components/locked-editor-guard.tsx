// apps/builder/src/features/templates/components/locked-editor-guard.tsx for
// the SPA. Templates (and so View-only template installs) have no Rust
// domain yet, so no resource is locked: the editor opens unlocked, as the
// builder does when `listLockedIds` finds nothing.
import type { TemplateResourceKind } from "@chatbotx.io/database/partials"
import type { ReactNode } from "react"
import { LockedEditorProvider } from "./locked-editor"

type LockedEditorGuardProps = {
  workspaceId: string
  resourceKind: TemplateResourceKind
  resourceId: string
  children: ReactNode
}

export function LockedEditorGuard({ children }: LockedEditorGuardProps) {
  return <LockedEditorProvider locked={false}>{children}</LockedEditorProvider>
}
