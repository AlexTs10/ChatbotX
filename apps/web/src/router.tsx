// The route tree (code-based TanStack Router). Each route file names the
// builder page it ports (`// page: app/...`); `scripts/inventory.mjs` reads
// those markers for conversion/specs/P4-routes.tsv.
import type { QueryClient } from "@tanstack/react-query"
import {
  createRootRouteWithContext,
  createRoute,
  createRouter,
  lazyRouteComponent,
  parseSearchWith,
  redirect,
  stringifySearchWith,
} from "@tanstack/react-router"
import { sessionQuery } from "@/lib/session"
import { DefaultErrorPage, NotFoundPage, RootLayout } from "@/routes/root"

// Each page is its own chunk (the main bundle keeps the root layout only).
const auth = () => import("@/routes/auth")
const workspace = () => import("@/routes/workspace")
const flows = () => import("@/routes/flows")
const fields = () => import("@/routes/fields")
const sequences = () => import("@/routes/sequences")
const ai = () => import("@/routes/ai")
const settings = () => import("@/routes/settings")
const dashboard = () => import("@/routes/dashboard")
const ecommerce = () => import("@/routes/ecommerce")
const tools = () => import("@/routes/tools")
const toolEditors = () => import("@/routes/tool-editors")
const comments = () => import("@/routes/comments")
const appointments = () => import("@/routes/appointments")
const booking = () => import("@/routes/booking")

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

// `/invitations/$code` is public too (lib/public-routes.ts).
const invitationRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "invitations/$code",
  component: lazyRouteComponent(
    () => import("@/routes/invitations"),
    "InvitationsPage",
  ),
})

// `/booking/*` is public too: the signed token is the authority.
const bookingPages = (
  [
    ["booking/picker", "BookingPickerPage"],
    ["booking/range-picker", "BookingRangePickerPage"],
    ["booking/schedule", "BookingSchedulePage"],
    ["booking/cancel", "BookingCancelPage"],
    ["booking/$slug", "BookingSlugPage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => rootRoute,
    path,
    component: lazyRouteComponent(booking, page),
  }),
)

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
  createRoute({
    getParentRoute: () => workspaceRoute,
    path: "inbox",
    component: lazyRouteComponent(() => import("@/routes/inbox"), "InboxPage"),
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
// A layout route with one index page (`<name>/layout.tsx` + `page.tsx`).
function sectionRoute<T extends Record<string, unknown>>(
  path: string,
  load: () => Promise<T>,
  layout: keyof T & string,
  page: keyof T & string,
) {
  const section = createRoute({
    getParentRoute: () => workspaceRoute,
    path,
    component: lazyRouteComponent(load, layout),
  })
  return section.addChildren([
    createRoute({
      getParentRoute: () => section,
      path: "/",
      component: lazyRouteComponent(load, page),
    }),
  ])
}
const sectionRoutes = [
  sectionRoute("tags", fields, "TagsLayout", "TagsPage"),
  sectionRoute(
    "custom-fields",
    fields,
    "CustomFieldsLayout",
    "CustomFieldsPage",
  ),
  sectionRoute("sequences", sequences, "SequencesLayout", "SequencesPage"),
]

// `dashboard/layout.tsx`: the analytics section (inbox cards + sub-pages).
const dashboardRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  path: "dashboard",
  component: lazyRouteComponent(dashboard, "DashboardLayout"),
})
const dashboardRoutes = dashboardRoute.addChildren(
  (
    [
      ["/", "DashboardIndexPage"],
      ["contacts", "DashboardContactsPage"],
      ["conversations", "DashboardConversationsPage"],
      ["ads", "DashboardAdsPage"],
      ["ads/$channel", "DashboardAdsPage"],
    ] as const
  ).map(([path, page]) =>
    createRoute({
      getParentRoute: () => dashboardRoute,
      path,
      component: lazyRouteComponent(dashboard, page),
    }),
  ),
)

// `(e-commerce)`: a pathless layout (ecommerce permission, tabs, shop link);
// `products/*`: the product forms outside it.
const ecommerceRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  id: "e-commerce",
  component: lazyRouteComponent(ecommerce, "EcommerceLayout"),
})
const ecommercePages = (
  [
    ["products", "ProductsPage"],
    ["product-categories", "ProductCategoriesPage"],
    ["product-vendors", "ProductVendorsPage"],
    ["orders", "OrdersPage"],
    ["orders/$orderId", "OrdersPage"],
    ["discount-codes", "DiscountCodesPage"],
    ["discount-codes/$discountCodeId", "DiscountCodesPage"],
    ["ecommerce-settings", "EcommerceSettingsPage"],
    ["ecommerce-settings/$section", "EcommerceSettingsPage"],
    ["ecommerce-analytics", "EcommerceAnalyticsPage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => ecommerceRoute,
    path,
    component: lazyRouteComponent(ecommerce, page),
  }),
)
const productFormsRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  id: "product-forms",
  component: lazyRouteComponent(ecommerce, "ProductsLayout"),
})
const productFormPages = (
  [
    ["products/create", "CreateProductPage"],
    ["products/$id/edit", "EditProductPage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => productFormsRoute,
    path,
    component: lazyRouteComponent(ecommerce, page),
  }),
)

// Comment / story automations: `(has-folder)/<x>/layout.tsx` is a pathless
// folder layout around the list; the create / edit / analytics pages sit
// outside it.
const commentListRoutes = (
  [
    ["fb-comments", "FbCommentsLayout", "FbCommentsPage"],
    ["ig-comments", "IgCommentsLayout", "IgCommentsPage"],
    ["ig-stories", "IgStoriesLayout", "IgStoriesPage"],
  ] as const
).map(([path, layout, page]) => {
  const folder = createRoute({
    getParentRoute: () => workspaceRoute,
    id: `${path}-folder`,
    component: lazyRouteComponent(comments, layout),
  })
  return folder.addChildren([
    createRoute({
      getParentRoute: () => folder,
      path,
      component: lazyRouteComponent(comments, page),
    }),
  ])
})
const commentPages = (
  [
    ["fb-comments/create", "CreateFbCommentPage"],
    ["fb-comments/$id", "EditFbCommentPage"],
    ["fb-comments/$id/analytics", "FbCommentAnalyticsPage"],
    ["ig-comments/create", "CreateIgCommentPage"],
    ["ig-comments/$id", "EditIgCommentPage"],
    ["ig-comments/$id/analytics", "IgCommentAnalyticsPage"],
    ["ig-stories/create", "CreateIgStoryPage"],
    ["ig-stories/$id", "EditIgStoryPage"],
    ["threads-comments", "ThreadsCommentsPage"],
    ["threads-comments/create", "CreateThreadsCommentPage"],
    ["threads-comments/$id", "EditThreadsCommentPage"],
    ["tiktok-comments", "TiktokCommentsPage"],
    ["tiktok-comments/create", "CreateTiktokCommentPage"],
    ["tiktok-comments/$id", "EditTiktokCommentPage"],
    ["tiktok-comments/$id/analytics", "TiktokCommentAnalyticsPage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => workspaceRoute,
    path,
    component: lazyRouteComponent(comments, page),
  }),
)

// The tools lists; `pipelines/layout.tsx` adds the contacts-access check.
const logs = () => import("@/routes/logs")
const logPages = (
  [
    ["error-logs", "ErrorLogsPage"],
    ["audit-logs", "AuditLogsPage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => workspaceRoute,
    path,
    component: lazyRouteComponent(logs, page),
  }),
)

const toolPages = (
  [
    ["polls", "PollsPage"],
    ["questionnaires", "QuestionnairesPage"],
    ["reflinks", "ReflinksPage"],
    ["qr-codes", "QrCodesPage"],
    ["minigames", "MinigamesPage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => workspaceRoute,
    path,
    component: lazyRouteComponent(tools, page),
  }),
)
const toolEditorPages = (
  [
    ["questionnaires/$id/edit", "EditQuestionnairePage"],
    ["questionnaires/$id/applicants", "QuestionnaireApplicantsPage"],
    ["polls/create", "CreatePollPage"],
    ["polls/$id/edit", "EditPollPage"],
    ["polls/$id/analytics", "PollAnalyticsPage"],
    ["qr-codes/create", "CreateQrCodePage"],
    ["qr-codes/$id/edit", "EditQrCodePage"],
    ["minigames/create", "CreateMinigamePage"],
    ["minigames/$id/edit", "EditMinigamePage"],
    ["minigames/$id/history", "MinigameHistoryPage"],
    ["reflinks/$reflinkId/analytics", "ReflinkAnalyticsPage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => workspaceRoute,
    path,
    component: lazyRouteComponent(toolEditors, page),
  }),
)
const pipelinesRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  path: "pipelines",
  component: lazyRouteComponent(tools, "PipelinesLayout"),
})
const pipelinesRoutes = pipelinesRoute.addChildren([
  createRoute({
    getParentRoute: () => pipelinesRoute,
    path: "/",
    component: lazyRouteComponent(tools, "PipelinesPage"),
  }),
  createRoute({
    getParentRoute: () => pipelinesRoute,
    path: "$id",
    component: lazyRouteComponent(toolEditors, "PipelineBoardPage"),
  }),
])

const appointmentCalendarsRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  path: "appointment-calendars",
  component: lazyRouteComponent(appointments, "AppointmentCalendarsLayout"),
})
const appointmentCalendarsRoutes = appointmentCalendarsRoute.addChildren(
  (
    [
      ["/", "AppointmentCalendarsPage"],
      ["$id/edit", "AppointmentCalendarEditPage"],
      ["appointments", "AppointmentsPage"],
      ["external", "ExternalCalendarsPage"],
    ] as const
  ).map(([path, page]) =>
    createRoute({
      getParentRoute: () => appointmentCalendarsRoute,
      path,
      component: lazyRouteComponent(appointments, page),
    }),
  ),
)

// `(ai)`: a pathless layout (flows permission, AI integration check).
const aiRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  id: "ai",
  component: lazyRouteComponent(ai, "AILayout"),
})
const aiPages = (
  [
    ["ai-agents", "AIAgentsPage"],
    ["ai-files", "AIFilesPage"],
    ["ai-files/test-query", "AIFilesTestQueryPage"],
    ["ai-files/$aiFileId", "AIFileDetailPage"],
    ["ai-functions", "AIFunctionsPage"],
    ["ai-mcp-servers", "AIMcpServersPage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => aiRoute,
    path,
    component: lazyRouteComponent(ai, page),
  }),
)

// `(settings)/settings`: the settings tabs (superAdmin).
const settingsRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  path: "settings",
  component: lazyRouteComponent(settings, "SettingsLayout"),
})
const settingsPage = (
  parent: () => typeof settingsRoute,
  path: string,
  page: keyof Awaited<ReturnType<typeof settings>>,
) =>
  createRoute({
    getParentRoute: parent,
    path,
    component: lazyRouteComponent(settings, page),
  })
const settingsChannelsRoute = settingsPage(
  () => settingsRoute,
  "channels",
  "SettingsChannelsLayout",
)
const settingsIntegrationsRoute = settingsPage(
  () => settingsRoute,
  "integrations",
  "SettingsIntegrationsLayout",
)
const settingsRoutes = settingsRoute.addChildren([
  settingsPage(() => settingsRoute, "/", "SettingsIndexPage"),
  settingsPage(() => settingsRoute, "general", "SettingsGeneralPage"),
  settingsPage(() => settingsRoute, "admins", "SettingsAdminsPage"),
  settingsPage(() => settingsRoute, "inbox-teams", "SettingsInboxTeamsPage"),
  createRoute({
    getParentRoute: () => settingsRoute,
    path: "inbox-teams/working-hours/$workingHoursId",
    component: lazyRouteComponent(appointments, "WorkingHoursEditorPage"),
  }),
  settingsPage(
    () => settingsRoute,
    "user-persistent-menus",
    "SettingsUserPersistentMenusPage",
  ),
  settingsPage(
    () => settingsRoute,
    "user-persistent-menus/create",
    "SettingsUserPersistentMenuCreatePage",
  ),
  settingsPage(
    () => settingsRoute,
    "user-persistent-menus/$menuId",
    "SettingsUserPersistentMenuEditPage",
  ),
  settingsChannelsRoute.addChildren([
    createRoute({
      getParentRoute: () => settingsChannelsRoute,
      path: "/",
      component: lazyRouteComponent(settings, "SettingsChannelsIndexPage"),
    }),
    createRoute({
      getParentRoute: () => settingsChannelsRoute,
      path: "$channel",
      component: lazyRouteComponent(settings, "SettingsChannelPage"),
    }),
  ]),
  settingsIntegrationsRoute.addChildren([
    createRoute({
      getParentRoute: () => settingsIntegrationsRoute,
      path: "/",
      component: lazyRouteComponent(settings, "SettingsIntegrationsIndexPage"),
    }),
    createRoute({
      getParentRoute: () => settingsIntegrationsRoute,
      path: "$slug",
      component: lazyRouteComponent(settings, "SettingsIntegrationPage"),
    }),
  ]),
])

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

// `app/admin/*`: the platform-operator console (platform admin only).
const admin = () => import("@/routes/admin")
const adminRoute = createRoute({
  getParentRoute: () => protectedRoute,
  path: "admin",
  component: lazyRouteComponent(admin, "AdminLayout"),
})
const adminPages = (
  [
    ["/", "AdminIndexPage"],
    ["workspaces", "AdminWorkspacesPage"],
    ["admins", "AdminAdminsPage"],
    ["help-items", "AdminHelpItemsPage"],
    ["platform-credentials", "AdminNotAvailablePage"],
    ["workspaces/deleted", "AdminNotAvailablePage"],
    ["platform-channels", "AdminNotAvailablePage"],
    ["default-template", "AdminNotAvailablePage"],
    ["audit-logs", "AdminNotAvailablePage"],
    ["smtp", "AdminNotAvailablePage"],
    ["branding", "AdminNotAvailablePage"],
    ["email-templates", "AdminNotAvailablePage"],
    ["email-templates/$type", "AdminNotAvailablePage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => adminRoute,
    path,
    component: lazyRouteComponent(admin, page),
  }),
)

const routeTree = rootRoute.addChildren([
  authRoute.addChildren(authPages),
  invitationRoute,
  ...bookingPages,
  protectedRoute.addChildren([
    homeRoute,
    flowDetailRoute,
    createChannelRoute,
    adminRoute.addChildren(adminPages),
    workspaceRoute.addChildren([
      ...workspacePages,
      flowsRoute.addChildren(flowsPages),
      ...sectionRoutes,
      dashboardRoutes,
      ecommerceRoute.addChildren(ecommercePages),
      productFormsRoute.addChildren(productFormPages),
      ...toolPages,
      ...logPages,
      ...toolEditorPages,
      ...commentListRoutes,
      ...commentPages,
      pipelinesRoutes,
      appointmentCalendarsRoutes,
      aiRoute.addChildren(aiPages),
      settingsRoutes,
    ]),
  ]),
])

export function createAppRouter(queryClient: QueryClient) {
  return createRouter({
    routeTree,
    context: { queryClient },
    defaultPreload: false,
    scrollRestoration: true,
    // Next.js search params are plain strings. The default JSON parse turns
    // an id above 2^53 into a rounded number (`useSearchParams` then reads
    // `…683` as `…684`); values that round-trip exactly still become numbers.
    parseSearch: parseSearchWith((value) => value),
    stringifySearch: stringifySearchWith(JSON.stringify),
  })
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof createAppRouter>
  }
}
