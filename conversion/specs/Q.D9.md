# Q.D9 — Appointments (calendars, external calendars, reminders, working hours)

Branch `feat/conversion-d9` (from `conversion` at `e2d16a491`). Sources:
`packages/business/src/appointment*`, `apps/builder/src/features/appointment-*`,
`external-calendars`, `booking-webview`, `apps/worker/src/{default,schedule}/handlers/*appointment*`,
`apps/worker/src/integration/handlers/appointment-scheduling.ts`,
`integrations/google-calendar`.

## 1. Slices

| # | Slice | Rust | State |
|---|-------|------|-------|
| 1 | Repository | `crates/db/src/repo/appointment.rs` (appointments + reminder dispatches; the calendar repo existed) | done |
| 2 | Business | `crates/business/src/appointment/{calendar,booking,external,reminder}.rs`: calendar CRUD / duplicate / update (default flows on create), slot generation (window, availability, buffer, caps, busy merge), booking / cancel / delete, webview completion, schedule detail, inbox booking link, reminders (schedule, reschedule, cancel, due scan, send), Google Calendar (token refresh, freeBusy, events insert / delete, revoke) | done |
| 3 | Public `/v1` routes | `crates/api/src/handlers/appointments.rs`: all 17 `appointmentCalendars.*`, `appointmentExternalCalendars.*`, `appointmentReminders.list`, `appointments.*` routes | done |
| 4 | `/rpc` | `crates/api/src/rpc_procedures/appointments.rs`: `appointmentCalendarsAPI.{listAppointmentCalendarsForFlowAPI,getContactAvailabilityAPI}`, `appointmentsAPI.listContactAppointmentsAPI`; 11 actions (calendar create / delete / duplicate / rename / active / update, book, cancel, delete, inbox booking link, Google disconnect) | done |
| 5 | Worker | `crates/worker/src/appointment.rs`: `sendAppointmentReminder`, `syncExternalCalendarEvent` (default queue), `scanAppointmentReminders` cron; `steps/appointment.rs`: the `appointmentScheduling` step (5 modes); the appointment cancel postback in `inbound.rs`; the appointment `{{variables}}` sources in `variables.rs` | done |
| 6 | Google OAuth connect (`connectGoogleCalendarAction`, callback), `updateGoogleCalendarIdAction` | - | left |
| 7 | Booking webview pages (`/booking/{slug,picker,range-picker,schedule,cancel}` + their 3 submit actions) | business functions done (`complete_webview_booking`, `cancel_by_token`, `schedule_detail`); pages / public actions not ported (no public page in the SPA yet) | left |
| 8 | Working hours CRUD (`workingHours.*` 10 routes, 7 actions, service) | the read side (`steps/working_hours.rs`) existed | left |
| 9 | Web SPA appointment screens (calendar list / edit, appointments list) | the SPA has no route for them yet; the inbox / contact / flow-editor parts call the ported procedures and actions | left |

## 2. Tests

`crates/api/tests/appointments_public_api.rs` (11 tests, adapted from the
public-route tests: real database instead of mocked services), unit tests in
`crates/business/src/appointment/*.rs` (window bounds, slot slicing, interval
merge, reminder send-at with DST) and `crates/worker/src/appointment.rs`
(attendee parsing, event id). The other TEST_MAP rows (webview, OAuth,
working hours, jsdom) stay `todo` with their slices.

## 3. Decisions (also in `DECISIONS.md`)

- One SQLite write transaction replaces the slot / cap / day advisory locks;
  the external busy re-check runs before that write.
- Reminder jobs are not removed from the queue (the Enqueuer has no remove);
  the send checks the dispatch status. Ceiling: after a calendar timezone
  change, a "days" reminder already queued fires at the old time.
- No distributed locks in `sendDispatch`, `syncExternalCalendarEvent` and the
  scan cron; the job ids keep one job per dispatch / operation.
- The Google auth is stored encrypted (AAD `integration-google-calendar:{id}`);
  plain rows are still read. The refresh uses the `google` platform credential
  of the credential owner.
- The `resultUsedByAI` availability rewrite (AI candidates loop) is not
  ported: the raw availability text is written.
- The cancel postback ends the inbound routing (the original falls through
  with no action left).
- The member conversation scope of the booking actions is the workspace check
  (as `messages.rs`); the contact scope uses the conversation assignee (as
  `contacts.rs`).

## 4. Parity suites to run

p1-04 `appointmentCalendars`, `appointmentExternalCalendars`,
`appointmentReminders`, `appointments`.
