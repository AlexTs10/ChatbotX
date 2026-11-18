// The browser-safe part of `packages/business/src/template/permissions.ts`
// (the business barrel is server only). The Rust server re-checks every rule.
import {
  type TemplateCategory,
  templateCategories,
} from "@chatbotx.io/database/partials"
import {
  hasWorkspacePermission,
  type PermissionsInput,
} from "@/lib/auth/permission-routes"

export const TEMPLATE_CATEGORY_PERMISSION = {
  flows: "flows",
  products: "ecommerce",
  aiFunctions: "flows",
  aiAgents: "flows",
  calendars: "flows",
  webchats: "flows",
  keywords: "flows",
  entryPointLinks: "flows",
  triggers: "flows",
  fbCommentAutomations: "flows",
  settings: "flows",
  sequences: "broadcast",
  aiMcpServers: "flows",
  aiFiles: "flows",
  questionnaires: "flows",
  couponTopics: "ecommerce",
  pipelines: "flows",
  customFields: "flows",
  tags: "flows",
  productCategories: "ecommerce",
} as const satisfies Record<TemplateCategory, string>

/** Managing (saving, sharing, deleting) templates and reading install history. */
export const TEMPLATE_MANAGE_PERMISSION = "flows" as const

export const canViewTemplateInstalls = (permissions: PermissionsInput) =>
  Object.values(TEMPLATE_CATEGORY_PERMISSION).some((key) =>
    hasWorkspacePermission(permissions, key),
  )

/** The categories a member can put in a template, in display order. */
export const listSelectableTemplateCategories = (
  permissions: PermissionsInput,
): TemplateCategory[] =>
  templateCategories.options.filter(
    (category) =>
      category !== "productCategories" &&
      hasWorkspacePermission(
        permissions,
        TEMPLATE_CATEGORY_PERMISSION[category],
      ),
  )
