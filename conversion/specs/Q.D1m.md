# Q.D1m — mini apps

Branch `feat/conversion-miniapps` (from `conversion` at `c16844719`). Follows
P3.6-D1 (`specs/P3.6-D1.md`).

## Slices

| # | Slice | Rust | Original |
|---|-------|------|----------|
| 1 | `packages/mini-app` server part: definition tree, Flow JSON import / export, validation and issue codes, display texts (map, escape), custom-field mappings, uploads | `core::mini_app` (`catalog`, `expression`, `runtime`, `serialize`, `validate`) | `packages/mini-app/src/*` |
| 2 | Domain: list / find / create / update / duplicate / delete, submissions (custom-field writes, upload swap), screen saves, uploads | `business::mini_app` | `packages/business/src/mini-app/*` |
| 3 | Public `/v1/mini-apps` (list, get, validate, create, duplicate, update, delete, bulk delete, flow-json, submissions) | `api::handlers::mini_apps` | `features/mini-apps/api/public.ts` |
| 4 | `/rpc`: `miniAppsAPI.listMiniAppOptionsAPI`, the page queries and the builder actions | `api::rpc_procedures::mini_apps` | `features/mini-apps/api/private.ts`, `queries`, `actions/*` |
| 5 | Public runtime: `POST /rpc/pages/miniApp`, `submitMiniAppAction`, `saveMiniAppScreenAction`, `POST /api/mini-apps/{id}/uploads`, flow resume | `server::mini_apps` | `app/mini-apps/page.tsx`, `lib/public-mini-app.ts`, `resolve-visitor-contact.ts`, `resolve-contact-variables.ts`, `resume-flow.ts`, `app/api/mini-apps/[id]/uploads/route.ts` |
| 6 | Flow step: Open Mini App buttons become signed `/mini-apps?id&token&resume` links | `worker::mini_app_buttons`, `worker::send_flow_step` | `apps/worker/src/lib/mini-app-buttons.ts` |
| 7 | Web SPA: list, editor, submissions and the public runner page | `apps/web/src/routes/mini-apps.tsx`, `features/mini-apps/*` (copied from the builder), `queries/index.ts` | `app/space/[workspaceId]/mini-apps/*`, `app/mini-apps/page.tsx` |

## Security

- Workspace routes: `/v1` uses the `mini-apps` token scope (write routes need
  a `full` token); `/rpc` uses the workspace membership check of
  `workspaceActionClient`. Every read and write is bound to the workspace id.
- Public runtime: the contact token must be signed for the Mini App's
  workspace (`verify_mini_app_token`); else a resume token for the same
  workspace and Mini App. Any other token gives an anonymous visitor.
- Resume: the claims must match the contact, workspace and Mini App, and the
  bot must be active. A KV once-guard (`mini-app-resume:{sha256(token)}`,
  8 days) stops a second resume; a failed enqueue frees the guard.
- Rate limits as the original: submit 10, screen save 60, upload 20 per
  window, per client IP (else contact, else anonymous).
- Uploads: cross-site posts are refused (403, fail closed), the body has a
  known bounded size (411 / 413), the Mini App must be enabled and its
  workspace servable.
- Contact values in display texts are escaped (no `${` / `{{` reference,
  markdown escaped only where the text is markdown, quotes escaped in a
  nested expression). React escapes the rendered HTML.

## Decisions

See `conversion/DECISIONS.md` (2026-11-17 "Q.D1m" lines).

## Tests

- `crates/core/tests/mini_app_{display_text,runtime,serialize,validate,validate_issue_codes}.rs`.
- `crates/business/tests/mini_app_definition.rs`, `mini_app_service.rs`, `mini_app_upload_service.rs`.
- `crates/api/tests/mini_apps_public_api.rs`.
- Unit tests: `server::mini_apps` (cross-site check), `worker::mini_app_buttons`
  (links, dropped buttons), `rpc_procedures::mini_apps` (number label).
- The submit and upload route tests are covered by parity `50-mini-apps`.

## Parity

Not run in this unit. Suites: `50-mini-apps`, `51-mini-apps-whatsapp`
(the WhatsApp publish cases are expected to fail: not ported).

## Not done

- WhatsApp Flows (publish, draft, preview, Flow removal on delete).
- The Messenger template Mini App button.
- The `resolveContactVariables` cases of
  `mini-app-contact-variables.test.ts`, the "files in the response message"
  cases of `validate.test.ts`.
