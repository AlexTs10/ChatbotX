import {
  Alert,
  AlertDescription,
  AlertTitle,
} from "@chatbotx.io/ui/components/ui/alert"
import { AlertTriangleIcon } from "lucide-react"
import { useTranslations } from "next-intl"

export function ScheduledDeletionBanner({ scheduled }: { scheduled: boolean }) {
  const t = useTranslations("workspace.deletion")
  if (!scheduled) {
    return null
  }

  return (
    <Alert variant="warning">
      <AlertTriangleIcon />
      <AlertTitle>{t("bannerTitle")}</AlertTitle>
      <AlertDescription>{t("bannerDescription")}</AlertDescription>
    </Alert>
  )
}
