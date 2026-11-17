import {
  Alert,
  AlertDescription,
  AlertTitle,
} from "@chatbotx.io/ui/components/ui/alert"
import { formatDate } from "@chatbotx.io/ui/lib/format"
import { ShieldCheckIcon } from "lucide-react"
import { useLocale, useTranslations } from "next-intl"

export function SupportAccessBanner({
  consentRequired,
  supportAccessUntil,
}: {
  consentRequired: boolean
  supportAccessUntil: Date | null
}) {
  const t = useTranslations("workspace.supportAccess")
  const locale = useLocale()
  const time = supportAccessUntil
    ? formatDate(supportAccessUntil, {
        hour: "numeric",
        minute: "numeric",
        locale,
      })
    : null

  return (
    <Alert variant="default">
      <ShieldCheckIcon />
      <AlertTitle>{t("bannerTitle")}</AlertTitle>
      <AlertDescription>
        {consentRequired && time
          ? t("bannerDescription", { time })
          : t("consentDisabled")}
      </AlertDescription>
    </Alert>
  )
}
