// The pages that `app/admin/*` (the root tenant, `scope="platform"`) and
// `app/manage/*` (the reseller's own tenant) share: branding, email
// templates, platform channels and platform credentials. The Rust reads
// re-check the caller (crates/api/src/rpc_procedures/admin.rs).
import type { ChannelType } from "@chatbotx.io/database/partials"
import { CREATABLE_CHANNELS } from "@chatbotx.io/database/partials"
import type { TenantModel } from "@chatbotx.io/database/types"
import { buttonVariants } from "@chatbotx.io/ui/components/ui/button"
import { ORPCError } from "@orpc/client"
import { useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import { ArrowLeftIcon } from "lucide-react"
import Link from "next/link"
import { useTranslations } from "next-intl"
import { type ReactNode, Suspense } from "react"
import {
  DEFAULT_ACCOUNT_CREDENTIALS_TEMPLATE,
  DEFAULT_FORGOT_PASSWORD_SUBJECT,
  DEFAULT_FORGOT_PASSWORD_TEMPLATE,
  DEFAULT_MAGIC_LINK_SUBJECT,
  DEFAULT_MAGIC_LINK_TEMPLATE,
  DEFAULT_SIGNUP_SUBJECT,
  DEFAULT_SIGNUP_TEMPLATE,
} from "../../../../packages/mail/src/emails/default-templates"
import { DEFAULT_ACCOUNT_CREDENTIALS_SUBJECT } from "../../../../packages/mail/src/emails/sign-up-credentials"
import { PlatformBrandingSettings } from "@/enterprise/features/platform-branding/platform-branding-settings"
import {
  type EmailTemplateType,
  emailTemplateTypes,
  storedEmailTemplateSchema,
} from "@/enterprise/features/platform-email-templates/email-template.schema"
import { PlatformEmailTemplateEditor } from "@/enterprise/features/platform-email-templates/platform-email-template-editor"
import { PlatformEmailTemplatesSettings } from "@/enterprise/features/platform-email-templates/platform-email-templates-settings"
import { PlatformChannelsSettings } from "@/features/platform-channels/platform-channels-settings"
import { ManagePlatformCredentials } from "@/features/platform-credentials/manage-platform-credentials"
import { serverFunction } from "@/lib/server-action"
import { NotFoundPage } from "@/routes/root"

/**
 * A console read: `{ value }`, or `null` when the server answers 404 (the
 * page's `notFound()`); `value` itself may be `null` (no row).
 */
export function useConsoleRead<T>(
  name: string,
  ...args: unknown[]
): { value: T } | null {
  return useSuspenseQuery({
    queryKey: ["spa", name, ...args],
    queryFn: async () => {
      try {
        return { value: (await serverFunction(name)(...args)) as T }
      } catch (error) {
        if (error instanceof ORPCError && error.status === 404) {
          return null
        }
        throw error
      }
    },
  }).data
}

/** The root (`platform`) or own tenant; the 404 page when refused. */
function TenantRead(props: {
  platform: boolean
  children: (tenant: TenantModel | null) => ReactNode
}) {
  const read = useConsoleRead<TenantModel | null>(
    props.platform ? "getRootTenant" : "getOwnTenant",
  )
  return read ? props.children(read.value) : <NotFoundPage />
}

function Title(props: { children: ReactNode }) {
  return <h3 className="font-bold text-lg sm:text-xl">{props.children}</h3>
}

export function BrandingPage(props: { platform: boolean }) {
  const t = useTranslations()
  return (
    <div className="space-y-4">
      <Title>{t("platformBranding.title")}</Title>
      <Suspense>
        <TenantRead platform={props.platform}>
          {(setting) => (
            <PlatformBrandingSettings
              scope={props.platform ? "platform" : "tenant"}
              setting={setting}
            />
          )}
        </TenantRead>
      </Suspense>
    </div>
  )
}

export function EmailTemplatesPage(props: { platform: boolean }) {
  const t = useTranslations()
  return (
    <div className="space-y-4">
      <Title>{t("platformEmailTemplates.title")}</Title>
      <Suspense>
        <TenantRead platform={props.platform}>
          {(setting) => (
            <PlatformEmailTemplatesSettings
              basePath={
                props.platform
                  ? "/admin/email-templates"
                  : "/manage/email-templates"
              }
              setting={setting}
            />
          )}
        </TenantRead>
      </Suspense>
    </div>
  )
}

type TemplateConfig = {
  titleKey: string
  descriptionKey: string
  variables: string[]
  settingKey:
    | "signupEmailTemplate"
    | "forgotPasswordEmailTemplate"
    | "magicLinkEmailTemplate"
    | "accountCredentialsEmailTemplate"
  defaultSubject: string
  defaultBody: string
}

/** `TEMPLATE_CONFIG` of the `email-templates/[type]` pages. */
const TEMPLATE_CONFIG: Record<EmailTemplateType, TemplateConfig> = {
  signup: {
    titleKey: "platformEmailTemplates.sections.signup.title",
    descriptionKey: "platformEmailTemplates.sections.signup.description",
    variables: ["{{userName}}", "{{verificationUrl}}", "{{brandName}}"],
    settingKey: "signupEmailTemplate",
    defaultSubject: DEFAULT_SIGNUP_SUBJECT,
    defaultBody: DEFAULT_SIGNUP_TEMPLATE,
  },
  forgotPassword: {
    titleKey: "platformEmailTemplates.sections.forgotPassword.title",
    descriptionKey:
      "platformEmailTemplates.sections.forgotPassword.description",
    variables: ["{{userName}}", "{{resetPasswordUrl}}", "{{brandName}}"],
    settingKey: "forgotPasswordEmailTemplate",
    defaultSubject: DEFAULT_FORGOT_PASSWORD_SUBJECT,
    defaultBody: DEFAULT_FORGOT_PASSWORD_TEMPLATE,
  },
  magicLink: {
    titleKey: "platformEmailTemplates.sections.magicLink.title",
    descriptionKey: "platformEmailTemplates.sections.magicLink.description",
    variables: ["{{userName}}", "{{magicUrl}}", "{{brandName}}"],
    settingKey: "magicLinkEmailTemplate",
    defaultSubject: DEFAULT_MAGIC_LINK_SUBJECT,
    defaultBody: DEFAULT_MAGIC_LINK_TEMPLATE,
  },
  accountCredentials: {
    titleKey: "platformEmailTemplates.sections.accountCredentials.title",
    descriptionKey:
      "platformEmailTemplates.sections.accountCredentials.description",
    variables: [
      "{{userName}}",
      "{{loginEmail}}",
      "{{initialPassword}}",
      "{{signInUrl}}",
      "{{brandName}}",
    ],
    settingKey: "accountCredentialsEmailTemplate",
    defaultSubject: DEFAULT_ACCOUNT_CREDENTIALS_SUBJECT,
    defaultBody: DEFAULT_ACCOUNT_CREDENTIALS_TEMPLATE,
  },
}

export function EmailTemplatePage(props: { platform: boolean }) {
  const t = useTranslations()
  const { type } = useParams({ strict: false }) as { type?: string }
  if (!emailTemplateTypes.includes(type as EmailTemplateType)) {
    return <NotFoundPage />
  }
  const templateType = type as EmailTemplateType
  const config = TEMPLATE_CONFIG[templateType]
  const title = t(config.titleKey as Parameters<typeof t>[0])
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <Link
          className={buttonVariants({ variant: "outline", size: "icon" })}
          href={
            props.platform ? "/admin/email-templates" : "/manage/email-templates"
          }
        >
          <ArrowLeftIcon className="size-4 rtl:rotate-180" />
          <span className="sr-only">{t("actions.back")}</span>
        </Link>
        <Title>{title}</Title>
      </div>
      <Suspense>
        <TenantRead platform={props.platform}>
          {(setting) => {
            const stored = storedEmailTemplateSchema.parse(
              setting?.[config.settingKey] ?? null,
            )
            return (
              <PlatformEmailTemplateEditor
                description={t(config.descriptionKey as Parameters<typeof t>[0])}
                key={templateType}
                scope={props.platform ? "platform" : "tenant"}
                template={{
                  subject: stored?.subject ?? config.defaultSubject,
                  body: stored?.body ?? config.defaultBody,
                }}
                title={title}
                type={templateType}
                variables={config.variables}
              />
            )
          }}
        </TenantRead>
      </Suspense>
    </div>
  )
}

/**
 * `filterPreviewChannels(CREATABLE_CHANNELS)`: `PREVIEW_CHANNELS` is empty,
 * so every creatable channel is offered.
 */
const OFFERED_CHANNELS: ChannelType[] = [...CREATABLE_CHANNELS]

function RootChannels() {
  return (
    <TenantRead platform>
      {(root) => (
        <PlatformChannelsSettings
          hiddenChannels={(root?.hiddenChannels ?? []) as ChannelType[]}
          offeredChannels={OFFERED_CHANNELS}
          scope="platform"
        />
      )}
    </TenantRead>
  )
}

function OwnChannels() {
  const read = useConsoleRead<{
    hiddenChannels: ChannelType[]
    platformHiddenChannels: ChannelType[]
  }>("getOwnPlatformChannels")
  if (!read) {
    return <NotFoundPage />
  }
  return (
    <PlatformChannelsSettings
      hiddenChannels={read.value.hiddenChannels}
      offeredChannels={OFFERED_CHANNELS}
      platformHiddenChannels={read.value.platformHiddenChannels}
    />
  )
}

export function PlatformChannelsPage(props: { platform: boolean }) {
  const t = useTranslations()
  return (
    <div className="space-y-4">
      <Title>{t("channels.title")}</Title>
      <Suspense>{props.platform ? <RootChannels /> : <OwnChannels />}</Suspense>
    </div>
  )
}

export function PlatformCredentialsPage(props: { platform: boolean }) {
  const t = useTranslations()
  return (
    <div className="space-y-4">
      {props.platform ? (
        <div className="space-y-1">
          <Title>{t("platformAdmin.platformCredentials.title")}</Title>
          <p className="text-muted-foreground text-sm">
            {t("platformAdmin.platformCredentials.description")}
          </p>
        </div>
      ) : (
        <Title>{t("platformCredentials.title")}</Title>
      )}
      <Suspense>
        <ManagePlatformCredentials
          scope={props.platform ? "platform" : "user"}
        />
      </Suspense>
    </div>
  )
}
