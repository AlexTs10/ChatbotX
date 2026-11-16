// `next/navigation` on TanStack Router (the copied builder code imports it).
import { useQueryClient } from "@tanstack/react-query"
import {
  notFound as routerNotFound,
  redirect as routerRedirect,
  useParams as useRouterParams,
  useRouterState,
  useRouter as useTanstackRouter,
} from "@tanstack/react-router"
import { useMemo } from "react"

type NavigateOptions = { scroll?: boolean }

export type AppRouterInstance = {
  push: (href: string, options?: NavigateOptions) => void
  replace: (href: string, options?: NavigateOptions) => void
  back: () => void
  forward: () => void
  refresh: () => void
  prefetch: (href: string) => void
}

export function useRouter(): AppRouterInstance {
  const router = useTanstackRouter()
  const queryClient = useQueryClient()
  return useMemo(
    () => ({
      push: (href) => router.history.push(href),
      replace: (href) => router.history.replace(href),
      back: () => router.history.back(),
      forward: () => router.history.forward(),
      // The server components re-ran on refresh; here the route loaders and
      // every query refetch.
      refresh: () => {
        router.invalidate()
        queryClient.invalidateQueries()
      },
      prefetch: () => undefined,
    }),
    [router, queryClient],
  )
}

export function usePathname(): string {
  return useRouterState({ select: (state) => state.location.pathname })
}

export function useSearchParams(): URLSearchParams {
  const search = useRouterState({ select: (state) => state.location.searchStr })
  return useMemo(() => new URLSearchParams(search), [search])
}

export type ReadonlyURLSearchParams = URLSearchParams

export function useParams<
  T extends Record<string, string | string[] | undefined> = Record<
    string,
    string
  >,
>(): T {
  return (useRouterParams as (options: { strict: false }) => unknown)({
    strict: false,
  }) as T
}

/** The first path segment below the layout (only `route-accordion-shell` uses it). */
export function useSelectedLayoutSegment(): string | null {
  const pathname = usePathname()
  return pathname.split("/").filter(Boolean).at(-1) ?? null
}

export function notFound(): never {
  throw routerNotFound()
}

export function redirect(href: string): never {
  throw routerRedirect({ href })
}

export const permanentRedirect = redirect

export function unstable_rethrow(_error: unknown): void {
  // Next.js rethrows its control-flow errors here; TanStack errors need nothing.
}
