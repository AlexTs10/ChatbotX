# Q.D13b — dynamic images, decisions, leaderboards, flow / dashboard listeners, places near me, bot simulator

Branch `feat/conversion-r2-d13b` (from conversion `672ea7e5f`). Follows
`Q.D13.md` ("Not done" list).

## Done

1. **Dynamic images** — `business::dynamic_image` (CRUD, PNG renderer in
   Rust: tiny-skia + ttf-parser + bundled fonts, QR codes), the `/v1`
   handlers, the SPA procedures (`rpc_procedures/dynamic_images.rs`), the
   public render route (`server/src/dynamic_images.rs`), the flow use; the
   screens `dynamic-images`, `/create`, `/$id/edit`.
2. **Decisions** — `business::decision` (profiles, connections, provider
   transport pinned to the SSRF-checked address, Global Router functions),
   `/v1` handlers, SPA procedures, the worker `decision` step; the screens
   `tools`, `tools/decision-profiles`, settings decision connections.
3. **Leaderboards and the reaction award** —
   - worker `leaderboard.rs`: the `leaderboard` queue (award / reverse /
     rebuildRank no-op / triggerRewards), the follower check (Instagram
     stored `followsBusiness`, Zalo, Telegram `getChatMember`, 10 min kv
     cache), the award sources (Zalo message, WhatsApp CTWA, Instagram story
     mention / emoji story reply, reactions, comments on messenger /
     instagram / tiktok / threads), the `changeGamificationScore` step, the
     `finishOverdueLeaderboards` cron, the export job, the Telegram
     `botChatStatus` / audience events.
   - business: `has_rule`, the participants CSV export (`run_export`).
   - api: SPA procedures (`rpc_procedures/leaderboards.rs`), public pages
     (`game_public.rs`): `pages.leaderboardPublic`, the `board` / `me`
     polling routes (ETag), `pages.minigamePlay`, `playMinigameAction`
     (guest rate limit).
   - web: leaderboards list / create / edit / participants, the public
     `/leaderboards` and `/minigames` pages.
4. **Flow-click and dashboard listeners** (worker `analytics_events.rs`, on
   the `events` worker): `flow:clicked` (broadcast-ops, sequence-ops,
   flow-ops, magic-link-stats, comment-automation-ops), `flow:ref`
   (ref-link-stats) and `analytics:dashboard` (contact / conversation /
   message / bot-received / agent-replied rows; bot-message quota, workspace
   usage and period ledger). A failed write returns its entry ids for retry.
5. **Places near me and bot simulator** — SPA procedures
   (`rpc_procedures/places.rs`), the `places-near-me` and `bot-simulator`
   screens, `botSimulator.getLink` (`/v1/bot-simulator/link`), the public
   preview `/bs/$workspaceId/$webchatId` (`pages.botSimulatorPreview`).

Tests: see `TEST_MAP.md` rows tagged `Q.D13b` (new:
`api/tests/{dynamic_images_public_api,decisions_public_api,rpc_leaderboard_actions,bot_simulator_public_api}.rs`,
`business/tests/leaderboard_service.rs`, `worker/tests/analytics_events.rs`,
unit tests in `worker/src/leaderboard.rs`, `worker/src/analytics_events.rs`,
`business/src/integration_webchat.rs`).

Decisions: `DECISIONS.md` lines tagged `Q.D13b`.

## Parity suites that cover this unit

- p1-08 `01-dynamic-images`
- p1-07 `21-decision-connections`, `22-decision-profiles`
- p1-10 `50-leaderboards-crud`, `51-leaderboards-participants`,
  `52-leaderboards-export-rewards`
- p1-13 `20-places`
- p1-04 `06-bot-simulator`

## Left (and why)

- **`runRef` handler** (`integration/handlers/ref.ts`): ref links (flows,
  drafts, QR codes, reflinks, minigame share, leaderboard referral credit,
  polls). No unit owns it; inbound still queues the job (ponytail in
  `inbound.rs`). The leaderboard referral credit and the TEST_MAP rows
  `run-ref-*`, `referral-ref-encoding`, `referral-utils-poll` wait for it.
- **Email open / click awards**: the email-topic tracking routes are not
  ported, so there is no hook (`leaderboard-email-telegram-sources` row).
- **Questionnaire / WhatsApp-flow survey awards**: those engines are not
  ported.
- **Dashboard producers**: the listener is in place, but most
  `analytics:dashboard` events (contact / conversation / message) are not
  emitted by Rust code yet; they belong to the domains that own those writes.
- **Leaderboard worker tests** `leaderboard-follower-check`,
  `leaderboard-jobs`, `change-gamification-score-step`,
  `leaderboard-welcome-flow`: mock-wiring tests; the paths run in parity
  p1-10. Jsdom UI tests (`leaderboards-admin-ui`, places UI) are not ported
  (no jsdom harness in the SPA gate).
