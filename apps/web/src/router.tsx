// The route tree (code-based TanStack Router). Each route file names the
// builder page it ports (`// page: app/...`); `scripts/inventory.mjs` reads
// those markers for conversion/specs/P4-routes.tsv.
import type { QueryClient } from "@tanstack/react-query"
import {
  type AnyRoute,
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
const ads = () => import("@/routes/ads")
const webhookTools = () => import("@/routes/webhook-tools")
const comments = () => import("@/routes/comments")
const appointments = () => import("@/routes/appointments")
const booking = () => import("@/routes/booking")
const shop = () => import("@/routes/shop")

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

// `/mini-apps` (the Mini App runner) is public too: contacts open it.
const miniApps = () => import("@/routes/mini-apps")
const publicMiniAppRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "mini-apps",
  component: lazyRouteComponent(miniApps, "PublicMiniAppPage"),
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

// `/shop/$workspaceId/*` is public too: the signed shop token is the authority.
const shopPages = (
  [
    ["shop/$workspaceId", "ShopHomePage"],
    ["shop/$workspaceId/categories", "ShopCategoriesPage"],
    ["shop/$workspaceId/products/$productId", "ShopProductPage"],
    ["shop/$workspaceId/cart", "ShopCartPage"],
    ["shop/$workspaceId/orders/$orderId", "ShopOrderPage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => rootRoute,
    path,
    component: lazyRouteComponent(shop, page),
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
  ...(
    [
      ["contacts/import", "ContactImportPage"],
      ["contacts/import/histories", "ContactImportHistoriesPage"],
      ["contacts/scan", "ContactScanPage"],
      ["contacts/scan/histories", "ContactScanHistoriesPage"],
    ] as const
  ).map(([path, page]) =>
    createRoute({
      getParentRoute: () => workspaceRoute,
      path,
      component: lazyRouteComponent(
        () => import("@/routes/contact-imports"),
        page,
      ),
    }),
  ),
  createRoute({
    getParentRoute: () => workspaceRoute,
    path: "agents",
    component: lazyRouteComponent(() => import("@/routes/agents"), "AgentsPage"),
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
  sectionRoute(
    "email-topics",
    () => import("@/routes/email-topics"),
    "EmailTopicsLayout",
    "EmailTopicsPage",
  ),
]

// `sequences/create`, `sequences/$id`: outside the folder layout.
const sequencePages = (
  [
    ["sequences/create", "CreateSequencePage"],
    ["sequences/$id", "SequenceDetailPage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => workspaceRoute,
    path,
    component: lazyRouteComponent(sequences, page),
  }),
)

// `broadcasts/layout.tsx` (broadcast permission) and its pages.
const broadcasts = () => import("@/routes/broadcasts")
const broadcastsRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  path: "broadcasts",
  component: lazyRouteComponent(broadcasts, "BroadcastsLayout"),
})
const broadcastsRoutes = broadcastsRoute.addChildren(
  (
    [
      ["/", "BroadcastsPage"],
      ["create", "CreateBroadcastPage"],
      ["$broadcastId/edit", "EditBroadcastPage"],
      ["$broadcastId/runs", "BroadcastRunsPage"],
    ] as const
  ).map(([path, page]) =>
    createRoute({
      getParentRoute: () => broadcastsRoute,
      path,
      component: lazyRouteComponent(broadcasts, page),
    }),
  ),
)

// `dashboard/layout.tsx`: the analytics section (inbox cards + sub-pages).
const dashboardRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  path: "dashboard",
  component: lazyRouteComponent(dashboard, "DashboardLayout"),
})
const dashboardRoutes = dashboardRoute.addChildren([
  ...(
    [
      ["/", "DashboardIndexPage"],
      ["contacts", "DashboardContactsPage"],
      ["conversations", "DashboardConversationsPage"],
    ] as const
  ).map(([path, page]) =>
    createRoute({
      getParentRoute: () => dashboardRoute,
      path,
      component: lazyRouteComponent(dashboard, page),
    }),
  ),
  ...(
    [
      ["ads", "DashboardAdsRedirectPage"],
      ["ads/google", "DashboardGoogleAdsPage"],
      ["ads/conversion-events", "DashboardConversionEventsPage"],
      ["ads/$channel", "DashboardAdsChannelPage"],
    ] as const
  ).map(([path, page]) =>
    createRoute({
      getParentRoute: () => dashboardRoute,
      path,
      component: lazyRouteComponent(ads, page),
    }),
  ),
])

// The ads tools and old ads bookmarks (routes/ads.tsx).
const adsPages = (
  [
    ["ads", "AdsPage"],
    ["ads/analytics", "AdsAnalyticsRedirectPage"],
    ["ads/connect-accounts", "AdsAnalyticsRedirectPage"],
    ["ads/conversion-events", "AdsConversionEventsRedirectPage"],
    ["messaging-ads", "MessagingAdsRedirectPage"],
    ["messaging-ads/$channel", "MessagingAdsChannelPage"],
    ["fb-lead-ads", "FbLeadAdsPage"],
    ["fb-lead-ads/create", "CreateFbLeadAdPage"],
    ["fb-lead-ads/$id", "EditFbLeadAdPage"],
    ["fb-marketing-messages", "FbMarketingMessagesPage"],
    ["fb-marketing-messages/create", "CreateFbMarketingMessagePage"],
    ["fb-marketing-messages/$id", "EditFbMarketingMessagePage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => workspaceRoute,
    path,
    component: lazyRouteComponent(ads, page),
  }),
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

// Keywords: `(has-folder)/<x>/layout.tsx` (super admin, folders, tabs) around
// the list; create / edit outside it (`routes/automated-responses.tsx`).
const automatedResponses = () => import("@/routes/automated-responses")
const automatedResponseListRoutes = (
  [
    [
      "automated-responses",
      "AutomatedResponsesLayout",
      "AutomatedResponsesPage",
    ],
    [
      "page-automated-responses",
      "PageAutomatedResponsesLayout",
      "PageAutomatedResponsesPage",
    ],
  ] as const
).map(([path, layout, page]) => {
  const folder = createRoute({
    getParentRoute: () => workspaceRoute,
    id: `${path}-folder`,
    component: lazyRouteComponent(automatedResponses, layout),
  })
  return folder.addChildren([
    createRoute({
      getParentRoute: () => folder,
      path,
      component: lazyRouteComponent(automatedResponses, page),
    }),
  ])
})
const automatedResponsePages = (
  [
    ["automated-responses/create", "CreateAutomatedResponsePage"],
    ["automated-responses/$id/edit", "EditAutomatedResponsePage"],
    ["page-automated-responses/create", "CreatePageAutomatedResponsePage"],
    ["page-automated-responses/$id/edit", "EditPageAutomatedResponsePage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => workspaceRoute,
    path,
    component: lazyRouteComponent(automatedResponses, page),
  }),
)

// `(has-folder)/triggers/layout.tsx` (super admin, folders) and the editor.
const triggers = () => import("@/routes/triggers")
const triggersRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  id: "triggers-folder",
  component: lazyRouteComponent(triggers, "TriggersLayout"),
})
const triggersRoutes = triggersRoute.addChildren([
  createRoute({
    getParentRoute: () => triggersRoute,
    path: "triggers",
    component: lazyRouteComponent(triggers, "TriggersPage"),
  }),
])
const editTriggerRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  path: "triggers/$id/edit",
  component: lazyRouteComponent(triggers, "EditTriggerPage"),
})

// `(coupons)/layout.tsx`: a pathless layout (tabs) around the two lists.
const coupons = () => import("@/routes/coupons")
const couponsRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  id: "coupons",
  component: lazyRouteComponent(coupons, "CouponsLayout"),
})
const couponsRoutes = couponsRoute.addChildren(
  (
    [
      ["coupons", "CouponsPage"],
      ["topic-coupons", "TopicCouponsPage"],
    ] as const
  ).map(([path, page]) =>
    createRoute({
      getParentRoute: () => couponsRoute,
      path,
      component: lazyRouteComponent(coupons, page),
    }),
  ),
)

// The tools lists; `pipelines/layout.tsx` adds the contacts-access check.
const logs = () => import("@/routes/logs")
const logPages = (
  [
    ["error-logs", "ErrorLogsPage"],
    ["audit-logs", "AuditLogsPage"],
    ["pricing", "PricingPage"],
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
// The super-admin tool screens (Q.D13a).
const webhookToolPages = (
  [
    ["inbound-webhooks", "InboundWebhooksPage"],
    ["inbound-webhooks/$id", "InboundWebhookEditorPage"],
    ["inbound-webhooks/$id/analytics", "InboundWebhookAnalyticsPage"],
    ["webhooks/$id/edit", "EditWebhookPage"],
    ["magic-links", "MagicLinksPage"],
    ["magic-links/$magicLinkId/analytics", "MagicLinkAnalyticsPage"],
    ["google-sheets", "GoogleSheetsPage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => workspaceRoute,
    path,
    component: lazyRouteComponent(webhookTools, page),
  }),
)
// `(has-folder)/webhooks/layout.tsx`: the folder store around the list (the
// edit page sits outside it).
const webhooksFolderRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  id: "webhooks-folder",
  component: lazyRouteComponent(webhookTools, "WebhooksLayout"),
})
const webhooksListRoute = webhooksFolderRoute.addChildren([
  createRoute({
    getParentRoute: () => webhooksFolderRoute,
    path: "webhooks",
    component: lazyRouteComponent(webhookTools, "WebhooksPage"),
  }),
])
const miniAppPages = (
  [
    ["mini-apps", "MiniAppsPage"],
    ["mini-apps/$id/edit", "EditMiniAppPage"],
    ["mini-apps/$id/submissions", "MiniAppSubmissionsPage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => workspaceRoute,
    path,
    component: lazyRouteComponent(miniApps, page),
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

// `(integrations)/whatsapps/[id]`, `messengers/[id]`, `instagrams/[id]`,
// `webchats/*`: the channel settings screens (superAdmin). The `calls` and
// `ads` tabs belong to the calling and ads units.
const channelSettings = () => import("@/routes/channel-settings")
const channelPage = <P extends AnyRoute>(
  parent: () => P,
  path: string,
  page: keyof Awaited<ReturnType<typeof channelSettings>>,
) =>
  createRoute({
    getParentRoute: parent,
    path,
    component: lazyRouteComponent(channelSettings, page),
  })
const whatsappChannelRoute = channelPage(
  () => workspaceRoute,
  "whatsapps/$id",
  "WhatsappLayout",
)
const messengerChannelRoute = channelPage(
  () => workspaceRoute,
  "messengers/$id",
  "MessengerLayout",
)
const instagramChannelRoute = channelPage(
  () => workspaceRoute,
  "instagrams/$id",
  "InstagramLayout",
)
const channelSettingsRoutes = [
  whatsappChannelRoute.addChildren(
    (
      [
        ["settings", "WhatsappSettingsPage"],
        ["account-healths", "WhatsappAccountHealthsPage"],
        ["flows", "WhatsappFlowsPage"],
        ["message-templates", "WhatsappMessageTemplatesPage"],
        ["useful-links", "WhatsappUsefulLinksPage"],
        ["ecommerce", "WhatsappEcommercePage"],
        ["automation", "WhatsappSettingsRedirect"],
        ["profile", "WhatsappSettingsRedirect"],
        ["capi", "WhatsappCapiRedirect"],
      ] as const
    ).map(([path, page]) => channelPage(() => whatsappChannelRoute, path, page)),
  ),
  messengerChannelRoute.addChildren(
    (
      [
        ["edit", "MessengerEditPage"],
        ["message-templates", "MessengerMessageTemplatesPage"],
        ["capi", "MessengerCapiRedirect"],
      ] as const
    ).map(([path, page]) => channelPage(() => messengerChannelRoute, path, page)),
  ),
  instagramChannelRoute.addChildren(
    (
      [
        ["edit", "InstagramEditPage"],
        ["capi", "InstagramCapiRedirect"],
      ] as const
    ).map(([path, page]) => channelPage(() => instagramChannelRoute, path, page)),
  ),
  channelPage(() => workspaceRoute, "webchats/create", "WebchatCreatePage"),
  channelPage(() => workspaceRoute, "webchats/$id/edit", "WebchatEditPage"),
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

const flowAnalyticsRoute = createRoute({
  getParentRoute: () => protectedRoute,
  path: "space/$workspaceId/flows/$id/analytics",
  component: lazyRouteComponent(
    () => import("@/routes/flow-detail"),
    "FlowAnalyticsPage",
  ),
})
const flowImportHistoriesRoute = createRoute({
  getParentRoute: () => protectedRoute,
  path: "space/$workspaceId/flows/import/histories",
  component: lazyRouteComponent(
    () => import("@/routes/contact-imports"),
    "FlowImportHistoriesPage",
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

// `(no-sidebar)/channels/<channel>/select`: the account pickers.
const channelSelectRoutes = (
  [
    ["channels/messenger/select", "MessengerSelectPage"],
    ["channels/instagram/select", "InstagramSelectPage"],
    ["channels/instagram-facebook/select", "InstagramFacebookSelectPage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => protectedRoute,
    path,
    component: lazyRouteComponent(() => import("@/routes/channel-select"), page),
  }),
)

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

// The template screens (`routes/templates.tsx`).
const templates = () => import("@/routes/templates")
const adminDefaultTemplateRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "default-template",
  component: lazyRouteComponent(templates, "AdminDefaultTemplatePage"),
})
const templatePages = (
  [
    ["templates", "TemplatesPage"],
    ["templates/create", "CreateTemplatePage"],
    ["templates/$id/edit", "EditTemplatePage"],
  ] as const
).map(([path, page]) =>
  createRoute({
    getParentRoute: () => workspaceRoute,
    path,
    component: lazyRouteComponent(templates, page),
  }),
)
const templateInstallsRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  path: "templates/installs",
  component: lazyRouteComponent(templates, "TemplateInstallsLayout"),
})
const templateInstallsRoutes = templateInstallsRoute.addChildren([
  createRoute({
    getParentRoute: () => templateInstallsRoute,
    path: "/",
    component: lazyRouteComponent(templates, "TemplateInstallsPage"),
  }),
  createRoute({
    getParentRoute: () => templateInstallsRoute,
    path: "$id",
    component: lazyRouteComponent(templates, "TemplateInstallationPage"),
  }),
])
// `/t/*` is public (lib/public-routes.ts); the install page asks for a sign-in.
const shareRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "t/$shareToken",
  component: lazyRouteComponent(templates, "PublicTemplateLayout"),
})
const shareRoutes = shareRoute.addChildren([
  createRoute({
    getParentRoute: () => shareRoute,
    path: "/",
    component: lazyRouteComponent(templates, "PublicTemplatePage"),
  }),
  createRoute({
    getParentRoute: () => shareRoute,
    path: "install",
    component: lazyRouteComponent(templates, "PublicTemplateInstallPage"),
  }),
])

const routeTree = rootRoute.addChildren([
  authRoute.addChildren(authPages),
  invitationRoute,
  shareRoutes,
  publicMiniAppRoute,
  ...bookingPages,
  ...shopPages,
  protectedRoute.addChildren([
    homeRoute,
    flowDetailRoute,
    flowAnalyticsRoute,
    flowImportHistoriesRoute,
    createChannelRoute,
    ...channelSelectRoutes,
    adminRoute.addChildren([...adminPages, adminDefaultTemplateRoute]),
    workspaceRoute.addChildren([
      ...workspacePages,
      flowsRoute.addChildren(flowsPages),
      ...sectionRoutes,
      broadcastsRoutes,
      ...sequencePages,
      dashboardRoutes,
      ecommerceRoute.addChildren(ecommercePages),
      productFormsRoute.addChildren(productFormPages),
      ...toolPages,
      ...templatePages,
      templateInstallsRoutes,
      ...logPages,
      ...toolEditorPages,
      ...webhookToolPages,
      webhooksListRoute,
      ...miniAppPages,
      ...commentListRoutes,
      ...adsPages,
      ...automatedResponseListRoutes,
      ...automatedResponsePages,
      triggersRoutes,
      editTriggerRoute,
      couponsRoutes,
      ...commentPages,
      pipelinesRoutes,
      appointmentCalendarsRoutes,
      aiRoute.addChildren(aiPages),
      settingsRoutes,
      ...channelSettingsRoutes,
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
