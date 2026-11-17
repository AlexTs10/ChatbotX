// page: app/layout.tsx
// page: app/not-found.tsx
// page: app/error.tsx
import { UiProvider } from "@chatbotx.io/ui"
import { Button, buttonVariants } from "@chatbotx.io/ui/components/ui/button"
import { DirectionProvider } from "@chatbotx.io/ui/components/ui/direction"
import { useSuspenseQuery } from "@tanstack/react-query"
import { type ErrorComponentProps, Outlet } from "@tanstack/react-router"
import { HouseIcon } from "lucide-react"
import Link from "next/link"
import { useTranslations } from "next-intl"
import { useEffect } from "react"
import { DataTableLabelsBridge } from "@/components/data-table-labels-bridge"
import { FormOptionalLabelBridge } from "@/components/form-optional-label-bridge"
import { TemplateInstallIntentGuard } from "@/features/templates/components/template-install-intent-guard"
import { TenantProvider } from "@/features/tenant"
import { getDirection } from "@/i18n/direction"
import { I18nProvider, locale } from "@/i18n/provider"
import { sessionQuery } from "@/lib/session"

export function RootLayout() {
  const { data } = useSuspenseQuery(sessionQuery)
  const { tenant } = data
  const dir = getDirection(locale)

  useEffect(() => {
    document.documentElement.lang = locale
    document.documentElement.dir = dir
    document.title = tenant.name
    document.body.className = tenant.theme
      ? `theme-${tenant.theme.toLowerCase()}`
      : ""
    if (tenant.faviconUrl) {
      document
        .querySelector<HTMLLinkElement>('link[rel="icon"]')
        ?.setAttribute("href", tenant.faviconUrl)
    }
  }, [dir, tenant])

  return (
    <TenantProvider settings={tenant}>
      <DirectionProvider direction={dir}>
        <UiProvider>
          <I18nProvider>
            <FormOptionalLabelBridge>
              <DataTableLabelsBridge>
                <TemplateInstallIntentGuard />
                <Outlet />
              </DataTableLabelsBridge>
            </FormOptionalLabelBridge>
          </I18nProvider>
        </UiProvider>
      </DirectionProvider>
    </TenantProvider>
  )
}

export function NotFoundPage() {
  const t = useTranslations()

  return (
    <main className="flex min-h-svh flex-col items-center justify-center gap-6 p-6 text-center">
      <p className="font-bold text-9xl text-muted-foreground/40 tabular-nums">
        404
      </p>

      <div className="flex max-w-md flex-col gap-2">
        <h1 className="font-semibold text-2xl tracking-tight">
          {t("notFound.title")}
        </h1>
        <p className="text-muted-foreground text-sm">
          {t("notFound.description")}
        </p>
      </div>

      <Link className={buttonVariants()} href="/">
        <HouseIcon className="size-4" />
        {t("notFound.backToHome")}
      </Link>
    </main>
  )
}

export function DefaultErrorPage({ error, reset }: ErrorComponentProps) {
  useEffect(() => {
    console.error(error)
  }, [error])

  return (
    <main className="flex h-full flex-col items-center justify-center">
      <h2 className="text-center">Something went wrong!</h2>
      <Button onClick={() => reset()}>Try again</Button>
    </main>
  )
}
