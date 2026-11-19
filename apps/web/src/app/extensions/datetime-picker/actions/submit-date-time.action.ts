// The client side of apps/builder/src/app/extensions/datetime-picker/actions/submit-date-time.action.ts:
// the Rust server serves it as the public `/rpc/pages/submitDateTime` (no login).
import { customAction, loadPage } from "@/lib/server-action"

export const submitDateTimeAction = customAction<{ completed: boolean }>(
  (input: { token: string; selectedValue: string }) =>
    loadPage("submitDateTime", input),
)
