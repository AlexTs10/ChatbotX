import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@chatbotx.io/ui/components/ui/card"
import { useTranslations } from "next-intl"
import { use } from "react"

export function QuestionnaireDashboardSummary({
  summary,
}: {
  summary: Promise<{
    totalApplicants: number
    completed: number
    completionRate: number
  }>
}) {
  const t = useTranslations()
  const data = use(summary)
  return (
    <div className="grid gap-3 md:grid-cols-3">
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">
            {t("questionnaires.applicants")}
          </CardTitle>
        </CardHeader>
        <CardContent className="font-semibold text-2xl">
          {data.totalApplicants}
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">
            {t("questionnaires.completed")}
          </CardTitle>
        </CardHeader>
        <CardContent className="font-semibold text-2xl">
          {data.completed}
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">
            {t("questionnaires.completionRate")}
          </CardTitle>
        </CardHeader>
        <CardContent className="font-semibold text-2xl">
          {data.completionRate}%
        </CardContent>
      </Card>
    </div>
  )
}
