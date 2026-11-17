# Q.D10 — Comments + posts

Branch `feat/conversion-d10` (from `conversion` at 7bf9f92c8, merged up to
e2d16a491). Follows Q.D2c (`commentAutomations.*`), Q.W2 (`low`
`replayMissedComment`) and Q.I1b (Meta channels + webhooks).

## Parts

| # | Part | Rust | Original |
|---|------|------|----------|
| 1 | `fbComments.*`, `igComments.*`, `igStories.*` CRUD (list, get, create, update, delete, deleteMany) | `business::comment_automation_crud`, `api::handlers::comment_crud` | `features/{fb,ig}-comments/api/public.ts`, `features/ig-stories/api/public.ts`, `comment-automation/service.ts`, `ig-story-automation/service.ts`, `comment-automation/account-scope.ts`, `database/partials/comment-automation.ts` |
| 2 | `igComments.listMedia`, `igStories.listStories`, Live `live_comments` re-subscription on create | `business::comment_automation_posts` | `ig-comments/lib/instagram-media.ts`, `ig-comments/lib/ensure-live-comments-subscription.ts`, `ig-stories/lib/instagram-stories.ts` |
| 3 | `threadsComments.*`, `tiktokComments.*` CRUD | `api::handlers::comment_channels`, `comment_automation_crud::build_channel_write` | `features/{threads,tiktok}-comments/{api/public.ts,schema/action.ts}`, the Threads / TikTok service builders |
| 4 | `incomingComment` integration job (Messenger: receive + every active automation of the account) | `worker::low::incoming_comment` (shares `receive_comment` with `replayMissedComment`) | `integration/handlers/received-message.ts` `receiveComment`, `comment-automation/index.ts` |

## Rules kept

- Request schemas as zod: unknown keys dropped, issues as `422
  invalidRequestData` with the zod paths; `name` trimmed (255, Threads /
  TikTok 120); `publicReply.values` at most 10; `mentionCount` 1–5;
  `aiModeration` refinement (agent + categories when enabled).
- Create with a `flow` reply: an unknown or foreign flow is 404 `Flow not
  found` (the template lock's `assertReferencable`); update: 422 `validation`.
- AI moderation agent must be an `AIAgent` of the workspace (422).
- `inboxId` must be an account of the automation's channel in the workspace
  (`assertAutomationAccount`, from the integration tables); `null` = every
  account; the stored value passes unchanged.
- Write normalisers: public text reply `value` / `values` mirrored; `hasGif`
  pinned off where the channel cannot see a GIF (Instagram, TikTok);
  `onlyCommentsWithImage` off on Instagram / TikTok; AI-moderation `delete`
  → `hide` on Threads; an Instagram Live post pins public reply, like,
  comment replies, hiding and reply delay off (`liveCommentCapabilities`),
  reading missing fields from the stored row on update.
- Story mention: `story` / `includeKeywords` pinned to `all` (only when
  present on update); `trigger` and `type` never change on update.
- Lists: `name` case-insensitive contains, `isActive`, `inboxId`, `folderId`
  (`"0"` = root), stories `trigger`; comment lists accept `sort[i][id|desc]`
  on any column; `perPage` 1–50; no default order.
- `deleteMany` ignores ids of another channel or workspace.
- Graph reads: fixed hosts (`graph.instagram.com` / `graph.facebook.com`),
  `limit=100`, `Authorization: Bearer`; a failing account is logged and left
  out of `posts` / `stories` but stays in `pages`. Live subscription: every
  Instagram Login account, best effort, never for `instagramFacebook`.

## Decisions

See `conversion/DECISIONS.md` (2026-11-17 "Q.D10"): no template edit guard
(`assertResourceEditable`, `withDeleteGuard`) as in the other ported
automation domains; Threads / TikTok resources are the stored row (no
`toThreadsResource` reshaping); `incomingComment` is Messenger-only.

## Tests

- Unit: `comment_automation_crud` (reply mirroring, Instagram pins + Live,
  Threads moderation, Threads / TikTok builders, story mention pins),
  `comment_automation_posts` (preview picture rule).
- Parity: p1-09 `30-ig-comments`, `31-ig-comments-media`, `40-ig-stories`
  (fixtures `createInstagramChannel`, `createInbox`, `addWorkspaceMember`
  ported to `parity/stacks/rust/ext/p1-09.ts`). p1-16 `threadsComments` has
  no parity tests on any branch.

## Left

- `fbComments.listPosts` (`/v1/fb-comments/facebook-posts`),
  `threadsComments.listPosts`: Graph / Threads post pickers (501 until
  ported).
- `incomingComment` for Instagram, Instagram-Facebook, Threads, TikTok
  (fails unrecoverable "comment automations are not ported"), the TikTok
  high-intent comment worker (`tiktokHighIntentComment`), story reply /
  mention automations in the inbound path.
- Low-queue comment actions Q.W2 left out (AI moderation, hide / like, image
  and mention filters, flow / AI replies, private-reply blocked events): the
  Messenger runner still sends text replies only.
- `/rpc` comment procedures (`fbCommentsAPI.*`, `igCommentsAPI.*`, …) for the
  SPA.
