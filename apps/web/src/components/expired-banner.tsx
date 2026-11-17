import {
  Alert,
  AlertDescription,
  AlertTitle,
} from "@chatbotx.io/ui/components/ui/alert"
import { buttonVariants } from "@chatbotx.io/ui/components/ui/button"
import { AlertTriangleIcon } from "lucide-react"
import Link from "next/link"
import { useTranslations } from "next-intl"

interface ExpiredBannerProps {
  blocked: boolean
  /** Why access is blocked; picks the banner copy. Defaults to the plan/trial message. */
  reason?: "status" | "mac" | null
}

export function ExpiredBanner({ blocked, reason }: ExpiredBannerProps) {
  const t = useTranslations(
    reason === "mac" ? "billing.macLimitReached" : "billing.trialExpired",
  )
  if (!blocked) {
    return null
  }

  return (
    <Alert className="border-amber-500/40 bg-amber-500/5" variant="warning">
      <AlertTriangleIcon />
      <AlertTitle>{t("bannerTitle")}</AlertTitle>
      <AlertDescription className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <p>{t("bannerDescription")}</p>
        <Link
          className={buttonVariants({ variant: "outline", size: "sm" })}
          href="/portal/pricing"
        >
          {t("bannerCta")}
        </Link>
      </AlertDescription>
    </Alert>
  )
}
