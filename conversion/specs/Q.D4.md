# Q.D4 — Templates (marketplace, installations, previews, template messages)

Branch `feat/conversion-d4` (from `conversion` at `993a50501`). The original
is `packages/business/src/template` (about 23k lines, 150 files), its
repositories in `packages/database/src/repositories/template*`, the builder
routes `features/templates/api/**`, its server actions
(`features/templates/actions/*`) and the worker template handlers. Its
tests are about 38k lines (141 class C files in TEST_MAP).

The unit is too large for one pass. Slice 1 ports what needs no snapshot /
install engine and what the SPA needs now; the other slices are listed under
"Left".

## Slices

| # | Slice | Rust | Original | Status |
|---|-------|------|----------|--------|
| 1 | Permissions, view-only / undeletable lookups, publisher reads and share settings, soft delete, installer list / settings / sync history; `/rpc` `templatesAPI.listLockedIdsAPI`; 13 `/v1` routes | `business::template::{permissions, service, installations}`, `api::handlers::templates`, `api::rpc_procedures::templates` | `permissions.ts`, `service.ts` (part), `installed-resource.service.ts` (`listLockedIds` / `listUndeletableIds`), `sync-request.service.ts` (`setAutoUpdate`, `setPublisherUninstallGrant`, `list`, `getDetail`), `features/templates/api/{index,public/templates,public/installations}.ts` | done |
| 2 | Snapshot save (`templates.create` / `update`, `template-save.ts`, `snapshot-*.ts`, every adapter's `collect`), selectable resources, dependency preview, version changes, fingerprints | `business::template::{save, collect, snapshot}`, `api::handlers::templates` | `template-save.ts`, `snapshot*.ts`, `selectable-resources.ts`, `adapters/**` | done |
| 3 | Install engine (`installation-request.service.ts` `request` / `retry`, `install.service.ts`, Phase R manifests, every adapter's `install`), `templateInstallations.install` / `retry` / `get` / `listResources`, `templatePreviews.get` (collisions), install worker job | `business::template::{install, request}`, `worker::template` | `install*.ts`, `adapters/manifests/**`, `apps/worker` install handler | done (no object staging; reinstall 501 until slice 4) |
| 4 | Sync (`sync-*.ts`), backups / restore, auto-update fan-out and its worker jobs | `business::template::{install::sync, syncs}`, `worker::template` | `sync-*.ts`, `apps/worker` sync / fan-out / redispatch handlers | done (no redispatch sweep, no idempotency key) |
| 5 | Uninstall (installer + publisher), cancel, uninstall worker jobs, `templates.listInstallers` / `getInstallerIssues` / `previewUninstallInstaller` / `uninstallInstaller` | `business::template::uninstall`, `api::handlers::templates`, `worker::template` | `uninstall*.ts`, `adapters/uninstall-*.ts` | done (one transaction, no producer effects / freeze / drain) |
| 6 | Tenant default template, template server actions for the SPA, edit / delete guards on the other domains' writers (`assertEditable`, `assertDeletable`) | | `tenant-default-template*`, `features/templates/actions/*` | left |

`templateMessages.list` (`/v1/template-messages`) is the deprecated alias of
`whatsappTemplates.list`: it returns the same rows with the WhatsApp
integration resource, so it is ported with `whatsappTemplates.list` (Q.D12),
as one alias line there.

## Slice 1 routes

`templates.list`, `get`, `delete`, `listLockedResources`, `getLink`,
`updateShare`, `rotateShare`, `listVersions`, `getAnalytics`;
`templateInstallations.list`, `update`, `listSyncs`, `getSync`; `/rpc`
`templatesAPI.listLockedIdsAPI`. Every other template route answers the
"not ported" 404.

## Security

- `/v1`: the route table gives each route the `templates` token scope; the
  workspace token middleware refuses a `read_only` token on every write
  except the two read POSTs (unchanged). The handlers build `tokenActor`
  from the token permission (`Principal.read_only`, new): `getLink`,
  `updateShare`, `rotateShare` and `delete` need a `full` token
  (`templateManagePermissionDenied`), `templateInstallations.update` refuses
  a `read_only` token.
- `/rpc`: the `/rpc` layer checks the session and the workspace membership,
  as `workspaceAuthorizedMidddleware`; `listLockedIdsAPI` needs no more.
- Every query is bound to the caller's workspace; another workspace's
  template, installation or sync is 404.
- Projections are explicit: no `payload`, `shareToken` (except on the link
  routes), `payloadHash`, `errorMessage`, `permissions` or claim fields.
- The share token is 32 random bytes (`ring`), base64url.
- No remote asset fetch in slice 1 (the cover image check is slice 2).

## Decisions

See `conversion/DECISIONS.md` (2026-11-17 "Q.D4" lines).

## Tests

- `crates/business/tests/template_service.rs`: ports of
  `integration/template-db.test.ts` "template lock precedence" (2 tests; the
  `excludeInstallationIds` step is the uninstall engine) and of
  `template-service-authorization.test.ts` (softDelete twice, the three
  manage guards incl. a `read_only` token, rotate), plus share expiry,
  list paging and auto-update states.
- `crates/api/tests/templates_public_api.rs`: ports of
  `templates-public-read-permissions.test.ts` (list paging, limit 101,
  getLink read_only / full, updateShare / rotateShare id and key),
  `templates-public-hardening.test.ts` (installation list projection, paging
  and page sizes 0 / 101) and `templates-public-scope.test.ts` (list without
  snapshot or selection); plus get / delete / lockedResources / syncs.
- `crates/api/tests/rpc_spa_procedures.rs`
  `template_locked_ids_answer_the_lock_badges`.
- Unit tests: permissions, cursor round trips, uninstall counts.

## Gate and parity

See the TRACKER row.
