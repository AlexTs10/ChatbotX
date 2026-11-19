# Q.D12-channels — channel screens, coexist, Messenger throttle

Branch `feat/conversion-r2-metach`. Base specs: `Q.D12.md`, `Q.I1a.md`,
`Q.I1b.md`.

## Done

### Goal 1 — channel screens (`P4-routes.tsv`)

- `(integrations)/whatsapps/*` (not `whatsapps/calls`), `messengers/[id]/*`,
  `instagrams/[id]/*`, `webchats/create`, `webchats/[id]/edit`: SPA routes in
  `apps/web/src/router.tsx`, features under `apps/web/src/features/integration-*`,
  page loaders in `crates/api/src/spa.rs`, server actions and procedures in
  `crates/api/src/rpc_procedures/channels.rs` (workspace member + the
  original role guard per action).
- `(no-sidebar)/channels/{messenger,instagram,instagram-facebook}/select`:
  `apps/web/src/routes/channel-select.tsx`, loader `pages.channelConnectSelect`
  (`resolveSelectSession`), connect procedures in
  `crates/api/src/rpc/channel_connect.rs`
  (`connectMessengerPageAPI`, `connectInstagramAccountAPI`,
  `connectInstagramFacebookAccountAPI`; `connectSessionCandidate`). The
  session must be the caller's own `connect` session, awaiting selection, in
  a workspace the caller is a member of.
- Instagram OAuth adapters (Instagram login and Instagram via Facebook) in the
  connect engine (`business::connection::oauth`); the OAuth state stays signed
  and bound to user and workspace (`connect_session.rs`); tokens are stored
  encrypted.

### Goal 2 — coexist

- `/v1` `messengerChannels.getCoexistStatus/setCoexist` and
  `instagramChannels.getCoexistStatus/setCoexist`
  (`handlers/whatsapp_coexist.rs`, channel from the route prefix) and `/rpc`
  `integrationMessengerAPIs.setCoexistMessengerAPI` /
  `integrationInstagramAPIs.setCoexistInstagramAPI`
  (`business::coexist::set_pull_coexist`: enable reuses a live run, else
  creates one with trigger `popup-enable`; disable fails live runs with
  "Coexist disabled").
- `coexistInstagramSync` (`worker/src/coexist_instagram.rs`): native
  Instagram (graph.instagram.com, v22.0, user-node profile read, name split)
  and Instagram via Facebook (graph.facebook.com, v23.0, Page node,
  `platform=instagram`); 90-day window with the 100-message floor; message
  detail fetch (limit 20); chunk budget 4 min with continuation jobs; terminal
  partial / failed / succeeded; provider error log rows on a fatal error; the
  AI-reads marker per `aiReadsSyncedHistory`.
- Coexist media for Messenger and Instagram (`worker/src/coexist_attachment.rs`):
  fresh URL from the Graph message (`getMessageMediaUrls`), positional pairing
  (same count and file type), live webhook rows stored under the channel
  media prefix (public read), the restore job (`restore: true`) that rewrites
  under the existing key and never marks the row on the final failed attempt.
  Downloads use `download_checked` (SSRF re-check at each hop).

### Goal 3 — Messenger sync throttle

- `worker/src/coexist_throttle.rs`: `X-Business-Use-Case-Usage` reading
  (`concurrencyForUsage`: 5 / 3 / 1 / 0, pause min(regain ?? 60, 300) s then
  concurrency 1) and the `X-App-Usage` reading for Instagram
  (`resolveUsageThrottle`: default / min(default, 3) / 1, 30 s pause capped at
  45 s). The conversations of a page run in parallel up to the current
  concurrency (`run_limited`, `pLimit` + `Promise.all`).
- `Upstream::send_with_headers` (default method, empty headers) gives the
  response headers to the pagers (`contact_scan::list_page_with_headers`,
  `graph_get_with_headers`).

### Goal 4 — TEST_MAP

32 stale rows of "WhatsApp channel", "Messenger channel", "Instagram
channel", "Coexist" and "Thread control" set to `ported` with their Rust test.

## Tests

- api: `rpc_channels.rs` (channel actions, coexist procedures),
  `rpc_channel_connect.rs` (connect session checks), `spa_pages.rs`.
- business: `connection_oauth_instagram.rs` (both Instagram adapters,
  business-account check, degraded subscribe).
- worker: `coexist_instagram_tests.rs`, `coexist_attachment.rs` tests,
  `coexist_throttle.rs` unit tests, `coexist_messenger.rs` tests.

## Gate (Dell, 2026-11-19)

fmt clean; clippy `-D warnings` clean (business, api, worker); `cargo test`
business + api + worker + server: 4373 passed, 0 failed. `pnpm --filter web
build` passes; `tsc --noEmit`: 45 errors (baseline 45, none new).

## Parity suites (not run here)

p1-10 / p1-11 WhatsApp + Meta: p1-11 `41`, `42`, `50`, `51`, `60`; p1-10 `20`,
`21`, `22`.

## Left (and why)

- WhatsApp calling / VoIP (`whatsapps/calls`): wacall unit.
- The three `ads` pages and the `capi` redirect target: ads unit.
- The media proxy route that enqueues `restore: true`: a separate route, not
  in this unit's pages.
- Instagram profile snapshot queue: no such queue in Rust yet
  (`ponytail:` note in `coexist_instagram.rs`).
- BUC header: serde_json orders the business buckets by key, JS by
  insertion; Meta sends one business id per Page token.
- `connectWhatsappNumberAPI`: not done in this unit.
- Branding / tag-sync follow-ups of the channel settings.
- About 256 class C TEST_MAP rows of these sections stay `todo` (mostly the
  TS integration tests: WhatsApp 66, Messenger 59, Instagram 60).
