# Q.F2-admin — admin, manage and public pages

Branch `feat/conversion-r2-admin` (from conversion 672ea7e5f).

## Goal 1 — web type errors

45 `error TS` fixed (commit 8af168405): `serverAction<Data>` typing in
`apps/web/src/lib/server-action.ts` (next-safe-action `[Data] extends [void]`
with `Data = any`), shims and the affected feature files.

## Goal 2 — `app/admin/*` and `app/manage/*`

- Web: `routes/admin.tsx`, `routes/manage.tsx`, the shared pages in
  `routes/platform-console.tsx`, `features/platform-credentials/manage-platform-credentials.tsx`.
- Rust reads (`crates/api/src/rpc_procedures/admin.rs`): `ADMIN_READS` (super
  admin; the `(enterprise)` ones also `assertEnterpriseFeatures`),
  `MANAGE_READS` (cloud + `hasOwnCredentialAccess`, reseller reads also
  `requireActiveReseller`), `getPlatformCredentialCards(scope)`,
  `listPlatformLogs` (`business::audit::log_query`), `getPlatformSmtpSettings`
  is 404 on cloud (`(non-cloud)/layout.tsx`).
- Actions: `previewEmailTemplate` (`compileEmailPreview`, MJML via `mrml`),
  `updatePlatformSmtp` verifies the server first, Stripe Connect webhook
  actions (`rpc/platform_credentials.rs`, `payments::stripe`).
- Tests: `crates/api/tests/platform_settings.rs`.

## Goal 3 — public pages

Loaders in `crates/api/src/spa/public_pages.rs` (`/rpc/pages/<name>`):
`unsubscribe`, `connectSession`, `landing` (QR / reflink and poll),
`botSimulator`, `datetimePicker`, `submitDateTime`, `webchat`; and
`GET /go/{workspaceId}` (server-rendered App Links document). Web:
`routes/public.tsx`. Tests: `crates/api/tests/spa_public_pages.rs`.

Left: `app/extensions/me` (needs the `systemFieldService` me-privacy port:
`getMePrivacyData`, `buildMeExport` + download route, `deleteMeData` with the
attachment deletes); webchat guest file upload (`/api/guest/messages` is JSON
only, P1.26).

## Goal 4 — D4 SPA dispatch test

`crates/api/tests/spa_default_template_dispatch.rs`: the first workspace of
`pages.home` stages the root default template and dispatches one
`installTemplate` job; a second visit does nothing.

## Parity suites

Not run here. Covering suites: see the final report of this unit.
