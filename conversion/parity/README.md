# Parity harness (Class A tests)

Black-box tests that must pass, unchanged, against the original TypeScript
stack and against the new Rust stack. Spec: `conversion/specs/P1.1.md`;
decisions: `conversion/DECISIONS.md` ("2026-11-11 — P1.1 harness decisions").

This folder is its own npm package (Node 24.22.1, vitest, `ws`, `pg`). It is
**not** in the pnpm workspace and imports nothing from `apps/`, `packages/` or
`integrations/`.

## Run against the original stack

On the Linux PC, from the repo root (Docker needed, no host Node needed):

```sh
bash conversion/parity/stacks/original/run-original.sh            # build, boot, test, down -v
bash conversion/parity/stacks/original/run-original.sh --no-build # use existing images
bash conversion/parity/stacks/original/run-original.sh --no-build -- tests/smoke/05-upstream.test.ts
```

From the Mac (copies the worktree to the PC first):

```sh
~/.local/bin/chatbotx-dell bash conversion/parity/stacks/original/run-original.sh --no-build
```

Options: `--build-only`, `--no-build`, `--keep` (leave the stack up),
`--up-only` (boot and exit), `--down` (stop and remove volumes),
`--update-snapshots` (vitest `-u`), `--stats` (one `docker stats` snapshot).
Arguments after `--` go to vitest.

What the script does: build images one at a time (fake-upstream, realtime,
worker, builder; times and sizes in `.state/build-times.txt`), start
postgres/redis/rustfs/mailhog/fake-upstream, start the builder (migrate + seed
+ serve), start worker, realtime and the Caddy edge, wait for each to be ready,
check inside builder, worker and realtime that `graph.facebook.com` resolves to
`1.1.1.1` and that HTTPS to it passes with the test CA, run vitest in the
`parity` runner container, write `.state/logs/stack.log`, then `down -v`
(unless `--keep`). The exit code is the vitest exit code.

Debug ports while the stack runs (PC loopback only): edge `127.0.0.1:18480`,
MailHog UI `127.0.0.1:18425`, fake upstream admin `127.0.0.1:18410`.

Reports: `.state/reports/junit.xml` and `results.json` (on the PC; `.state/`
is git-ignored and the rsync of `chatbotx-dell` keeps it).

### Snapshots

`expect(...).toMatchSnapshot()` values are recorded on the original stack and
committed (`tests/**/__snapshots__/`). The runner sets `CI=true`, so a missing
snapshot fails. To record one: run with `--update-snapshots`, then copy the
new files from the PC to the Mac **before** the next `chatbotx-dell` call (its
rsync deletes files that exist only on the PC):

```sh
rsync -av 'dell-5570:chatbotx-remote/ChatbotX-lean/conversion/parity/tests/' conversion/parity/tests/
```

Snapshots are frozen in P1.28. After that, a changed snapshot is a parity bug.

## Layout

```
package.json  package-lock.json  tsconfig.json  vitest.config.ts  .nvmrc
fake-upstream/          stack-agnostic fake third-party upstream (own image, no deps)
stacks/
  harness.compose.yml   fake-upstream, mailhog, parity runner, networks (all stacks)
  runner-entrypoint.sh  npm ci (when the lockfile changes) + vitest
  original/             compose.yml, original.env, Caddyfile, fixtures.ts, run-original.sh
src/
  config.ts  http.ts  rpc.ts  auth.ts  mailhog.ts  fake.ts  realtime.ts
  normalize.ts  eventually.ts  unique.ts  sign.ts  sequencer.ts  global-setup.ts
  fixtures/driver.ts    FixtureDriver interface
  fixtures/index.ts     loads stacks/$STACK/fixtures.ts; createTenant(), signedInClient()
tests/smoke/*.test.ts   P1.1 smoke tests (later units add tests/<group>/)
.state/                 git-ignored: reports/, logs/, build-times.txt, stats.txt
```

## Writing tests

- Use only `BASE_URL` (`config.baseUrl`), `RT_URL`, `FAKE_UPSTREAM_ADMIN_URL`,
  `MAILHOG_URL` and the fixture driver. Nothing stack-specific in `tests/`.
- Setup without a public HTTP path goes through `fixtures()` /
  `createTenant()` (direct DB rows). Assertions are HTTP, WebSocket, fake
  upstream records, mail and files only.
- Unique data per test: `uniqueEmail()`, `uniqueHost()` (under
  `*.parity.test`, covered by the fake certificate), `uniqueId()`. One new
  tenant (user + workspace + token) per test that needs one. The community
  edition allows one workspace per owner, so `createTenant()` makes a new user
  each time.
- Create a workspace before the first HTTP call that lists the owner's
  workspaces (the app caches that list; a direct insert cannot clear it).
- Async effects: `eventually(fn, {timeoutMs})`, `fake.waitFor(...)`,
  `waitForMail(...)`, `socket.waitFor(...)`. No fixed sleeps.
- Compare with a `Normalizer` (one per test): ids -> `<id:n>` (same id, same
  label), ISO times -> `<ts>` (checked against the test window), `cbx_ws_`/
  `cbx_api_` tokens, JWTs, `seq`, and signed URL query parameters. Compare
  headers through `comparableHeaders()` and errors through `comparableError()`.
- Auth endpoints allow 3 calls per 10 s per client address. Each `HttpClient`
  sends its own random `X-Forwarded-For`, so use a new client per simulated user.
- The last file, `tests/smoke/99-interception.test.ts`, fails when the stack
  made any third-party request that no script answered, or used a TLS name
  outside `fake-upstream/hosts.txt`. Script every upstream call a test causes.

## Fake upstream

One container, IP `1.1.1.1` on network `parity-upstream` (subnet 1.1.1.0/24),
also `fake-upstream` on `parity-internal`:

- DNS (UDP+TCP 53): every A query -> `1.1.1.1`, AAAA/other -> empty.
- HTTPS 443 and HTTP 80: records every request (method, host, path, query,
  headers, body), answers from the newest matching script, then built-in
  defaults (`fake-upstream/defaults.ts`: DoH on `1.1.1.1/dns-query`), else 599.
- TLS: `gen-certs.sh` makes a test CA and one leaf certificate (SANs from
  `hosts.txt`) at container start, in volume `parity-tls`. No key is committed;
  the CA key is deleted after signing; `down -v` removes the volume.
- Admin API on 4010: `POST /__admin/reset`, `POST /__admin/scripts`,
  `POST /__admin/dns` (per-name answers, e.g. a private IP for SSRF tests),
  `GET /__admin/requests`, `GET /__admin/requests/wait`, `GET /__admin/unmatched`,
  `GET /__admin/ca`, `GET /__admin/health`. Client: `src/fake.ts`.

## What a new stack must provide

Add `stacks/<name>/` with:

1. `compose.yml`, merged on top of `stacks/harness.compose.yml` (paths relative
   to `conversion/parity`). It must provide:
   - a service `app` on `parity-internal`, port 80, that serves the app,
     `/api/*`, `/rpc/*`, `/ws/*` (realtime, prefix stripped) and `/storage/*`
     (public files). Links in mail must use `http://app`. The edge passes the
     client's `X-Forwarded-For` through as the only value (see
     `original/Caddyfile`); auth rate limits are per that address;
   - mail to `mailhog:1025` (SMTP);
   - every process that makes outbound calls on both networks, with
     `dns: [1.1.1.1]` and the CA `/parity-tls/ca.crt` (volume `parity-tls`)
     trusted: `NODE_EXTRA_CA_CERTS` for Node, `SSL_CERT_FILE` for Rust with
     native roots (a Rust build that uses only `webpki-roots` ignores it);
   - the same app settings as `original/original.env` where they change
     behavior (community edition, `STORED_IMAGE_OPTIMIZATION=off`,
     `SCHEDULER_TICK_INTERVAL_MS=1000`, secrets, `BETTER_AUTH_URL`);
   - a `parity:` service override that sets `STACK: <name>` and the env the
     fixture adapter needs.
2. `fixtures.ts` exporting `createFixtureDriver(): FixtureDriver`
   (`src/fixtures/driver.ts`) that writes the same logical rows as
   `original/fixtures.ts` (user + password credential, workspace + owner
   member, workspace API token stored as SHA-256 hex) to that stack's database.
3. `run-<name>.sh` with the same steps and options as `run-original.sh`.

Then run the same `tests/` with it. A test may not branch on `STACK`.
