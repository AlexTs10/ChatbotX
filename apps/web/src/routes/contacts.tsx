// page: app/space/[workspaceId]/contacts/page.tsx
// `requireContactsAccess` is the Rust page loader `pages.contactsAccess`.
import { useSuspenseQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import { useTranslations } from "next-intl"
import { ContactsTable } from "@/features/contacts/contacts-table"
import { CreateContactDialog } from "@/features/contacts/create-contact-dialog"
import { loadPage } from "@/lib/server-action"

export function ContactsPage() {
  const { workspaceId } = useParams({ strict: false }) as {
    workspaceId: string
  }
  const t = useTranslations()
  const { data: contactPermissionScope } = useSuspenseQuery({
    queryKey: ["spa", "contactsAccess", workspaceId],
    queryFn: () =>
      loadPage<{ canViewEmailAndPhone: boolean }>("contactsAccess", {
        workspaceId,
      }),
  })

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-2">
        <h4 className="font-bold text-xl">{t("contacts.title")}</h4>
        <CreateContactDialog workspaceId={workspaceId} />
      </div>

      <ContactsTable
        canViewEmailAndPhone={contactPermissionScope.canViewEmailAndPhone}
        key={workspaceId}
        workspaceId={workspaceId}
      />
    </div>
  )
}
