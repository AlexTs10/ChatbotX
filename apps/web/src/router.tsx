// The route tree (code-based TanStack Router). Each route file names the
// builder page it ports (`// page: app/...`); `scripts/inventory.mjs` reads
// those markers for conversion/specs/P4-routes.tsv.
import type { QueryClient } from "@tanstack/react-query"
import {
  createRootRouteWithContext,
  createRoute,
  createRouter,
  lazyRouteComponent,
  redirect,
} from "@tanstack/react-router"
import { sessionQuery } from "@/lib/session"
import { DefaultErrorPage, NotFoundPage, RootLayout } from "@/routes/root"

// Each page is its own chunk (the main bundle keeps the root layout only).
const auth = () => import("@/routes/auth")
const workspace = () => import("@/routes/workspace")
const flows = () => import("@/routes/flows")

type RouterContext = { queryClient: QueryClient }

const rootRoute = createRootRouteWithContext<RouterContext>()({
  beforeLoad: ({ context }) =>
    context.queryClient.ensureQueryData(sessionQuery),
  component: RootLayout,
  notFoundComponent: NotFoundPage,
  errorComponent: DefaultErrorPage,
})

// `/auth/*` is public (lib/public-routes.ts).
const authRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "auth",
  component: lazyRouteComponent(auth, "AuthLayout"),
})
const authPages = [
  createRoute({
    getParentRoute: () => authRoute,
    path: "sign-in",
    component: lazyRouteComponent(auth, "SignInPage"),
  }),
  createRoute({
    getParentRoute: () => authRoute,
    path: "sign-up",
    component: lazyRouteComponent(auth, "SignUpPage"),
  }),
  createRoute({
    getParentRoute: () => authRoute,
    path: "forgot-password",
    component: lazyRouteComponent(auth, "ForgotPasswordPage"),
  }),
  createRoute({
    getParentRoute: () => authRoute,
    path: "reset-password",
    component: lazyRouteComponent(auth, "ResetPasswordPage"),
  }),
  createRoute({
    getParentRoute: () => authRoute,
    path: "magic-link-sent",
    component: lazyRouteComponent(auth, "MagicLinkSentPage"),
  }),
  createRoute({
    getParentRoute: () => authRoute,
    path: "change-password",
    // The page self-guards: no session → sign-in, no forced change → home.
    beforeLoad: async ({ context }) => {
      const { user } = await context.queryClient.ensureQueryData(sessionQuery)
      if (!user) {
        throw redirect({ href: "/auth/sign-in" })
      }
      if (!user.mustChangePassword) {
        throw redirect({ href: "/" })
      }
    },
    component: lazyRouteComponent(auth, "ChangePasswordPage"),
  }),
]

// Everything else needs a session (proxy.ts) and a current password
// (`enforcePasswordCurrent` in every protected layout).
const protectedRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: "protected",
  beforeLoad: async ({ context, location }) => {
    const { user } = await context.queryClient.ensureQueryData(sessionQuery)
    if (!user) {
      throw redirect({
        href: `/auth/sign-in?callbackURL=${encodeURIComponent(window.location.origin + location.href)}`,
      })
    }
    if (user.mustChangePassword) {
      throw redirect({ href: "/auth/change-password" })
    }
  },
})

const homeRoute = createRoute({
  getParentRoute: () => protectedRoute,
  path: "/",
  component: lazyRouteComponent(() => import("@/routes/home"), "MainPage"),
})

const workspaceRoute = createRoute({
  getParentRoute: () => protectedRoute,
  path: "space/$workspaceId",
  component: lazyRouteComponent(workspace, "WorkspaceLayout"),
})
const workspacePages = [
  createRoute({
    getParentRoute: () => workspaceRoute,
    path: "/",
    component: lazyRouteComponent(workspace, "WorkspaceIndexPage"),
  }),
  createRoute({
    getParentRoute: () => workspaceRoute,
    path: "contacts",
    component: lazyRouteComponent(
      () => import("@/routes/contacts"),
      "ContactsPage",
    ),
  }),
]
const flowsRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  path: "flows",
  component: lazyRouteComponent(flows, "FlowsLayout"),
})
const flowsPages = [
  createRoute({
    getParentRoute: () => flowsRoute,
    path: "/",
    component: lazyRouteComponent(flows, "FlowsPage"),
  }),
]

// `(no-sidebar)`: the flow editor is outside the workspace sidebar layout.
const flowDetailRoute = createRoute({
  getParentRoute: () => protectedRoute,
  path: "space/$workspaceId/flows/$id",
  component: lazyRouteComponent(
    () => import("@/routes/flow-detail"),
    "FlowDetailPage",
  ),
})

const createChannelRoute = createRoute({
  getParentRoute: () => protectedRoute,
  path: "channels/create",
  component: lazyRouteComponent(
    () => import("@/routes/channels"),
    "CreateChannelPage",
  ),
})

const routeTree = rootRoute.addChildren([
  authRoute.addChildren(authPages),
  protectedRoute.addChildren([
    homeRoute,
    flowDetailRoute,
    createChannelRoute,
    workspaceRoute.addChildren([
      ...workspacePages,
      flowsRoute.addChildren(flowsPages),
    ]),
  ]),
])

export function createAppRouter(queryClient: QueryClient) {
  return createRouter({
    routeTree,
    context: { queryClient },
    defaultPreload: false,
    scrollRestoration: true,
  })
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof createAppRouter>
  }
}
