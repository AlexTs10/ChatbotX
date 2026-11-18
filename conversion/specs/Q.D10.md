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
| 4 | `incomingComment` for Messenger, Instagram (both logins), Threads, TikTok: contact + comment row, then `processCommentAutomation` on the integration queue (`comment-auto-<id>`; one attempt on Threads / TikTok); a replay runs its one automation inline | `worker::low` (`receive_comment`, `process_comment_automation`) | `integration/handlers/received-message.ts` `receiveComment`, `comment-automation/index.ts` |
| 5 | The runner: schedule, post (Live), comment-reply, keyword, mention-count and image filters, new-contact / once-per-post / other-post dedup, like and rule hides (moderation log, unhide delay) through `changeChannelMessageState`, public text replies through `sendChannelMessage` (3 s spacing), private text replies on the channel (claim, 7-day / 48 h / Live window, blocked events), flow replies through `sendFlow` with the comment anchor | `worker::low::Run` | `comment-automation/{automation-matching,hide-comments,moderation,public-reply,private-reply,record,comment-tags,comment-attachment}.ts` |
| 6 | TikTok comment handlers (`sendComment`, `sendPrivateReply`, `deleteComment`, `likeComment`, `hideComment`), commenter lookup (self-authored skip, unknown author withheld), `tiktokHighIntentComment` (flags the stored comment) | `worker::channels::tiktok`, `worker::low::tiktok_high_intent_comment` | `integrations/tiktok/src/handlers/comment/*`, `tiktok-comment-identity.ts`, `tiktok-high-intent-comment.ts` |
| 7 | Story reply / mention automations from the inbound path (`processStoryReplyAutomation`, `processStoryMentionAutomation` on the AI queue) | `worker::story_automation` | `story-automation-dispatch.ts`, `story-reply-automation/index.ts` |
| 8 | `fbComments.listPosts`, `threadsComments.listPosts` (published / ads / reels per page; Threads posts without reposts) | `business::comment_automation_posts` | `fb-comments/lib/facebook-posts.ts`, `threads-comments/lib/threads-posts.ts` |
| 9 | Threads / TikTok answers reshaped (`toThreadsResource`, `toTiktokResource`) | `comment_automation_crud::channel_resource` | `features/{threads,tiktok}-comments/api/resource.ts` |
| 10 | `/rpc` queries, actions and private procedures of the five screens, and the SPA routes | `api::rpc_procedures::comments`, `apps/web/src/routes/comments.tsx` | `features/*-comments`, `features/ig-stories`, `features/shared/comment-automation` |

## AI moderation and AI replies (2026-11-18, `worker::low::comment_ai`)

| Part | Rust | Original |
|------|------|----------|
| `moderateCommentWithAI`: before the reply filters; queues `commentModeration` on the AI queue (`comment-moderation-<automation>-<comment>`, a replay adds `-replay-<id>`) and the automation waits (`pendingAI`, not a miss); fails open | `Run::moderate_with_ai` | `moderation.ts` |
| `processCommentModeration`: active automation + workspace agent, verdict cache (10 min, key = comment + agent + sorted categories + instruction), the agent's auto-reply models in order (`generateText` + `Output.object`, temperature 0), blocked categories only; a block records the `moderated` miss, the `ai` moderation log row and the hide / delete job (`showCommentsAfter` = none); anything else resumes `processCommentAutomation` for this automation with `moderationChecked` | `comment_moderation` | `moderation.ts` |
| `deleteChannelMessage` settles the moderation log row as the hide does | `chat::delete_message_from_channel` | `chat` handlers |
| AI-agent replies: the event row first (`replyText` null), then `commentAIReply` (`comment-ai-reply-<automation>-<comment>-<channel>`, the reply delay); a failed enqueue settles the row `failed` | `Run::dispatch_ai_reply` | `public-reply.ts`, `private-reply.ts`, `record.ts` |
| `processCommentAIReply`: skips (no text, outside active hours: dedup rollback + event dropped), failures (agent / conversation / contact inbox missing, no text: dedup rollback + event `failed`), Threads / TikTok comments in the untrusted-data envelope, public reply row + chat send, private DM on the channel (`markDelivered`, settle, inbox row; none of these fails the job); a terminal throw settles `failed` with the generated text | `comment_ai_reply` | `ai-reply.ts`, `dedup.ts` |
| `generateAIReplyText`: agent prompt with contact variables + the generate-only injection policy, auto-reply integrations only, no tools, one step | `generate_reply_text` | `automated-response/replies.ts`, `lib/ai/reply-model.ts` |
| Story `AIAgent` reply (mention: the fixed "mentioned your account" message) | `story_automation::process` | `story-reply-automation/index.ts` |

Security: the agent, automation, conversation and contact inbox are read
with the job's workspace id; a private reply's integration must belong to
that workspace; keys stay in the integration rows.

Tests: `comment_ai` unit tests (prompt / guard / cache key / verdict) and
`low_tests::ai_moderation_holds_the_reply_and_hides_a_flagged_comment`
(scripted provider).

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
automation domains. Later lines: Threads / TikTok resources are reshaped
as the original; the runner uses the channel trait and the chat /
integration queues.

## Tests

- Unit: `comment_automation_crud` (reply mirroring, Instagram pins + Live,
  Threads moderation, Threads / TikTok builders, story mention pins,
  resource shapes), `comment_automation_posts` (preview picture rule),
  `low` (matching, mentions, hide rules, delays; a replay through the
  queues and a recording channel; a queued run and a keyword miss),
  `story_automation` (story / keyword matching).
- Integration: `business/tests/comment_automation_post_lists.rs`,
  `api/tests/rpc_spa_procedures.rs` (the comment screens).
- Parity: p1-09 `30-ig-comments`, `31-ig-comments-media`, `40-ig-stories`
  (fixtures `createInstagramChannel`, `createInbox`, `addWorkspaceMember`
  ported to `parity/stacks/rust/ext/p1-09.ts`). p1-16 `threadsComments` has
  no parity tests on any branch.

## Left

- The AI-reply analytics keep the Rust counter rule of `record_event`
  (every channel counts its public replies; the original counts public
  replies only where the channel has no DM, `countsTowardStats`). The public
  AI reply is not settled by the chat worker on a terminal send failure
  (`settleCommentAutomationFailure`), as the other public replies.
- No replay priority (`withReplayPriority`) and no orphaned-integration
  cleanup (`runWithOrphanedIntegrationCleanup`) on the AI jobs.
- The deferred TikTok private reply (`deferredCommentPrivateReply`): a
  TikTok private reply is sent only when the high-intent flag is already on
  the comment; else it is logged and not sent.
- Live-comment pacing (`reserveLiveCommentDelay`), Messenger Live detection
  (the attachment lookup), the leaderboard award, post tracking
  (`channelPostService`), commenter avatars, comment attachments on the
  stored row, user-tag tracking (`trackUserTags`), the Graph fallback for a
  Messenger comment without `tags`.
- A private flow reply runs in the comment conversation, not a resolved DM
  conversation (`resolveDirectMessageConversationId`).
- The template edit guard (`assertResourceEditable`, `withDeleteGuard`).

