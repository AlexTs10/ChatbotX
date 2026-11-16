// The route tree (code-based TanStack Router). Each route file names the
// builder page it ports (`// page: app/...`); `scripts/inventory.mjs` reads
// those markers for conversion/specs/P4-routes.tsv.
import type { QueryClient } from "@tanstack/react-query"
import {
  createRootRouteWithContext,
  createRoute,
  createRouter,
  redirect,
} from "@tanstack/react-router"
import { sessionQuery } from "@/lib/session"
import {
  AuthLayout,
  ChangePasswordPage,
  ForgotPasswordPage,
  MagicLinkSentPage,
  ResetPasswordPage,
  SignInPage,
  SignUpPage,
} from "@/routes/auth"
import { ContactsPage } from "@/routes/contacts"
import { FlowsLayout, FlowsPage } from "@/routes/flows"
import { MainPage } from "@/routes/home"
import { DefaultErrorPage, NotFoundPage, RootLayout } from "@/routes/root"
import { WorkspaceIndexPage, WorkspaceLayout } from "@/routes/workspace"

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
  component: AuthLayout,
})
const authPages = [
  createRoute({
    getParentRoute: () => authRoute,
    path: "sign-in",
    component: SignInPage,
  }),
  createRoute({
    getParentRoute: () => authRoute,
    path: "sign-up",
    component: SignUpPage,
  }),
  createRoute({
    getParentRoute: () => authRoute,
    path: "forgot-password",
    component: ForgotPasswordPage,
  }),
  createRoute({
    getParentRoute: () => authRoute,
    path: "reset-password",
    component: ResetPasswordPage,
  }),
  createRoute({
    getParentRoute: () => authRoute,
    path: "magic-link-sent",
    component: MagicLinkSentPage,
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
    component: ChangePasswordPage,
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
  component: MainPage,
})

const workspaceRoute = createRoute({
  getParentRoute: () => protectedRoute,
  path: "space/$workspaceId",
  component: WorkspaceLayout,
})
const workspacePages = [
  createRoute({
    getParentRoute: () => workspaceRoute,
    path: "/",
    component: WorkspaceIndexPage,
  }),
  createRoute({
    getParentRoute: () => workspaceRoute,
    path: "contacts",
    component: ContactsPage,
  }),
]
const flowsRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  path: "flows",
  component: FlowsLayout,
})
const flowsPages = [
  createRoute({
    getParentRoute: () => flowsRoute,
    path: "/",
    component: FlowsPage,
  }),
]

const routeTree = rootRoute.addChildren([
  authRoute.addChildren(authPages),
  protectedRoute.addChildren([
    homeRoute,
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
