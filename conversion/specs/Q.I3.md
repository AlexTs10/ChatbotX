# Q.I3 — marketing / e-mail integrations

Branch `feat/conversion-i34` (from `conversion` at `e2d16a491`). Follows
Q.D2c slice 6 (the API-key connects and verify).

## Slices

| # | Slice | Rust | Original |
|---|-------|------|----------|
| 1 | Provider clients (lists, tags, fields, subscriber writes) for ActiveCampaign, Drip, GetResponse, Klaviyo, Mailchimp, MailerLite, Moosend, SendGrid | `business::email_marketing::<provider>` | `integrations/<name>/src/**` |
| 2 | Flow steps `activeCampaignSyncContact`, `dripSubscribeSubscriber`, `getResponseAddContact`, `klaviyoSyncProfile`, `mailchimpAddMember`, `mailerLiteAddSubscriber`, `moosendCreateContact`, `sendGridAddContact` (locks, retries, provider error log) | `business::email_marketing::<provider>::*_step`, `worker::steps::email_marketing` | `apps/worker/src/integration/handlers/*-handler.ts` |
| 3 | Flow-editor option lists `integration<Name>API.*` (`/rpc`) with the original error mapping | `api::rpc_procedures::email_marketing` | `features/integration-<name>/api/index.ts` |
| 4 | SMTP client (implicit TLS on 465, STARTTLS, PLAIN / LOGIN, header-injection checks), verify, the `email` flow step | `business::email_marketing::smtp`, `worker::steps::email_marketing::send_email` | `integrations/smtp`, `apps/worker/.../send-email.ts`, `convert-button.ts` |
| 5 | SMTP integration service, `/v1/smtp-integrations` (list / get / create / update / delete), `/rpc` `integrationSmtpAPI.listIntegrationSmtps`, actions `createSmtpAction` / `updateSmtpAction` / `deleteSmtpAction` | `business::email_marketing::smtp_service`, `api::handlers::smtp_integrations`, `api::rpc_procedures::email_marketing` | `packages/business/src/integration-smtp/service.ts`, `features/integration-smtp/**` |

## Security

- Every provider call goes through `SafeHttp`: the SSRF guard
  (`net::ssrf_guard::assert_public_url`) on the first URL and on every
  redirect hop; credentials only to the first origin; 15 s timeout.
- SMTP: a known provider pins host and port (`smtpHostMap`); CR / LF in
  `from`, `to` and `subject` is refused; the connection is verified before
  it is saved.
- Stored credentials are sealed with the process `Encryption`: the SMTP
  `auth` (the original stores it plain) and the API-key satellites (Q.D2c
  stored them plain; fixed here). Plain rows still read.
- The public SMTP resource and the update action never return `auth`.

## Simplifications (DECISIONS 2026-11-17 Q.I3)

- The email step renders a plain table HTML in place of MJML.
- Email-topic tracking (recipient rows, open pixel, signed click URLs)
  waits for Q.D5; buttons link straight to their URL.
- Locks and the Moosend quota are process-local (one server process).
- `new Date(text)` parsing is RFC 3339 / RFC 2822 only.

## Tests

Unit tests in each provider module (request shape, error mapping, retries,
FakeHttp), the SMTP module (a local fake SMTP server, header injection,
render) and the handler / rpc modules (input checks, error mapping).

## Parity

No P1 suite covers these routes yet (P1.15 `smtpIntegrations` is `todo`).
Run `p1-05-channels-connections` (the API-key connects now seal the auth)
and `smoke`.
