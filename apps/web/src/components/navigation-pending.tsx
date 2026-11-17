// apps/builder/src/components/navigation-pending.tsx hooked into Next.js
// internals (AppRouterContext, useLinkStatus) to show the destination
// skeleton during a server navigation. In the SPA a navigation renders at
// once and each page shows its own Suspense fallback, so these are
// pass-throughs.
// ponytail: no workspace-wide skeleton on navigation; add one on
// `useRouterState().status === "pending"` if route loaders get slow.
import type { ReactNode } from "react"

export function NavigationPendingProvider({
  children,
}: {
  children: ReactNode
}) {
  return children
}

export function NavigationPendingBoundary({
  children,
}: {
  children: ReactNode
}) {
  return children
}

export function LinkPendingReporter(_props: {
  href: string
  setPendingHref: (update: (current: string | null) => string | null) => void
}) {
  return null
}

export function LinkPendingSpinner() {
  return null
}
