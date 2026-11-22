use std::sync::Arc;

use super::*;
use crate::channel::RecordingChannel;
use crate::contact_scan::tests::{Graph, setup};

#[test]
fn matching_rules_follow_the_original() {
    let ids = json!({"type": "postIds", "value": ["1_22"]});
    assert!(match_post(&ids, "9_22", false));
    assert!(!match_post(&ids, "1_23", false));
    assert!(match_post(
        &json!({"type": "all", "value": []}),
        "1_23",
        false
    ));
    assert!(!match_post(
        &json!({"type": "all", "value": []}),
        "1_23",
        true
    ));
    assert!(!match_post(&json!({"type": "live"}), "1_23", false));
    assert!(match_post(&json!({"type": "live"}), "1_23", true));
    let all = json!({"type": "all", "value": []});
    assert!(match_keywords(
        &json!({"type": "contain", "value": ["PRÍCE"]}),
        &[],
        Some("what price?"),
        "contain"
    ));
    assert!(!match_keywords(
        &json!({"type": "equal", "value": ["price"]}),
        &[],
        Some("what price?"),
        "contain"
    ));
    assert!(!match_keywords(
        &all,
        &["spam".into()],
        Some("buy SPAM"),
        "contain"
    ));
    assert!(match_keywords(
        &all,
        &["spam".into()],
        Some("buy spam"),
        "equal"
    ));
    assert!(!is_comment_reply(Some("1_22"), "1_22", "22_5"));
    assert!(!is_comment_reply(Some("22"), "1_22", "22_5"));
    assert!(is_comment_reply(Some("22_4"), "1_22", "22_5"));
    assert_eq!(
        reply_texts(&json!({"type": "text", "value": " hi "})),
        vec!["hi"]
    );
    assert_eq!(
        reply_texts(&json!({"type": "text", "values": [{"value": "a"}, {"value": " "}]})),
        vec!["a"]
    );
    assert!(within_schedule(None, Some("10:00"), "UTC"));
    assert!(within_schedule(
        Some("00:00"),
        Some("23:59"),
        "Europe/Berlin"
    ));
    assert!(!will_send(&json!({"type": "flow", "value": null})));
    assert!(will_send(&json!({"type": "AIAgent", "value": "9"})));
    assert_eq!(
        compute_delay_ms(&json!({"type": "minutes", "value": 2})),
        120_000
    );
    assert!(compute_delay_ms(&json!({"type": "randomWithin5Minutes"})) < 300_000);
    assert_eq!(unhide_delay_ms("12h"), 43_200_000);
    assert_eq!(unhide_delay_ms("3d"), 3 * 86_400_000);
    assert_eq!(unhide_delay_ms("11d"), 0);
}

#[test]
fn mentions_and_hide_rules() {
    assert_eq!(
        text_mentions(Some("hi @Ann and @ann, @bob_1. mail a@b.c")),
        vec!["ann".to_owned(), "bob_1".to_owned()]
    );
    let tags = json!([{"id": "1"}, {"id": "1"}, {"id": "2"}]);
    assert_eq!(tag_ids(&tags), vec!["1".to_owned(), "2".to_owned()]);
    let none = Attachments::default();
    let hide = json!({"hasPhoneNumber": true, "hasEmoji": true, "hasKeywords": true, "keywords": ["SALE"]});
    assert_eq!(
        match_hide_rule(&hide, Some("call +49 170 1234567"), none),
        Some(("phoneNumber", None))
    );
    assert_eq!(
        match_hide_rule(&hide, Some("big sale"), none),
        Some(("keyword", Some("SALE".to_owned())))
    );
    assert_eq!(
        match_hide_rule(&hide, Some("nice 😀"), none),
        Some(("emoji", None))
    );
    assert_eq!(match_hide_rule(&hide, Some("nice"), none), None);
    let image = json!({"hasImage": true});
    let with_image = Attachments {
        image: true,
        ..none
    };
    assert_eq!(
        match_hide_rule(&image, None, with_image),
        Some(("image", None))
    );
    assert!(has_hide_rule(&json!({"showCommentsAfter": "6h"})));
    assert!(!has_hide_rule(&json!({"showCommentsAfter": "none"})));
}

async fn add_automation(deps: &Deps, sql: &'static str) {
    deps.db
        .write(move |tx| Ok(tx.execute_batch(sql)?))
        .await
        .unwrap();
}

fn comment(automation: Option<&str>) -> Value {
    let mut data = json!({
        "workspaceId": "2", "integrationType": "messenger", "integrationIdentifier": "p1",
        "commentData": { "commentId": "55_9", "postId": "p1_55", "parentId": "p1_55", "fromId": "u7",
            "fromName": "Ana Lima", "message": "How much?", "createdTime": now_ms() / 1000 - 60 },
    });
    if let Some(id) = automation {
        data["replay"] = json!({ "automationId": id });
    }
    data
}

/// p1-05 `07`: the replay stores the comment, likes and hides it through
/// the chat queue, queues the public reply, sends the private reply on the
/// channel and records both events.
#[tokio::test]
async fn a_replay_runs_the_automation_through_the_queues_and_the_channel() {
    let (mut deps, jobs) = setup().await;
    let channel = RecordingChannel::new();
    deps.channels =
        crate::Channels::builtin().with("messenger", channel.clone() as Arc<dyn Channel>);
    add_automation(
        &deps,
        r#"INSERT INTO "CommentAutomation" ("id", "name", "workspaceId", "type", "post", "publicReply", "privateReply", "options", "hideComments")
           VALUES (7, 'A', 2, 'messenger', '{"type":"postIds","value":["p1_55"]}', '{"type":"text","value":"Thanks {a|a}"}',
                   '{"type":"text","value":"DM"}', '{"likeUserComment":true}',
                   '{"hasKeywords":true,"keywords":["much"],"showCommentsAfter":"6h"}')"#,
    )
    .await;
    let graph = Graph(Default::default(), json!({}));
    replay_missed_comment(&deps, &graph, &comment(Some("7")))
        .await
        .unwrap();

    assert!(graph.0.lock().unwrap().is_empty());
    let queued = jobs.0.lock().unwrap().clone();
    let names: Vec<&str> = queued.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "changeChannelMessageState",
            "changeChannelMessageState",
            "changeChannelMessageState",
            "sendChannelMessage"
        ]
    );
    assert_eq!(
        queued[1].1.job_id.as_deref(),
        Some("moderate-comment-7-55_9")
    );
    assert_eq!(queued[2].1.delay_ms, Some(6 * 3_600_000));
    let calls = channel.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].handler, "sendPrivateReply");
    assert_eq!(calls[0].data["message"]["text"], "DM");
    assert_eq!(
        calls[0].data["message"]["contentAttributes"]["replyToCommentId"],
        "55_9"
    );

    let counts: (i64, i64, i64, i64, String) = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT (SELECT COUNT(*) FROM "CommentAutomationEvent" WHERE "commentId" = '55_9' AND "status" = 'sent'),
                          (SELECT "sentCount" FROM "CommentAutomation" WHERE "id" = 7),
                          (SELECT "moderatedCount" FROM "CommentAutomation" WHERE "id" = 7),
                          (SELECT COUNT(*) FROM "Message" WHERE "type" = 'comment'),
                          (SELECT "rule" FROM "CommentModerationLog" WHERE "commentId" = '55_9')"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )?)
        })
        .await
        .unwrap();
    // Messenger has a DM: only the private reply moves `sentCount`.
    assert_eq!(counts, (2, 1, 1, 2, "keyword".to_owned()));
}

/// `countsTowardStats`: private always; public only on Threads (no DM).
#[tokio::test]
async fn only_private_replies_count_where_the_channel_has_a_dm() {
    let (deps, _jobs) = setup().await;
    add_automation(
        &deps,
        r#"INSERT INTO "CommentAutomation" ("id", "name", "workspaceId", "type", "post")
           VALUES (7, 'M', 2, 'messenger', '{"type":"all","value":[]}'),
                  (9, 'T', 2, 'threads', '{"type":"all","value":[]}')"#,
    )
    .await;
    let counts = deps
        .db
        .read(|c| {
            Ok(
                [(7, "private"), (7, "public"), (9, "public"), (99, "public")]
                    .map(|(id, ch)| comment_ai::counts_toward_stats(c, id, ch).unwrap()),
            )
        })
        .await
        .unwrap();
    assert_eq!(counts, [true, false, true, false]);
}

/// `settleCommentAutomationFailure` / `Delivered`: the anchor on a reply
/// message names its event row.
#[tokio::test]
async fn a_terminal_send_failure_settles_the_anchored_event() {
    let (deps, _jobs) = setup().await;
    add_automation(
        &deps,
        r#"INSERT INTO "CommentAutomation" ("id", "name", "workspaceId", "type", "post")
           VALUES (9, 'T', 2, 'threads', '{"type":"all","value":[]}');
           INSERT INTO "CommentAutomationEvent" ("id", "workspaceId", "automationId", "postId", "commentId",
               "replyChannel", "replyType", "replyText", "status", "occurredAt")
           VALUES (1, 2, 9, 'p', 'c1', 'public', 'text', 'Hi', 'sent', 0)"#,
    )
    .await;
    let attributes = json!({
        "replyToCommentId": "c1",
        "commentAutomation": { "automationId": "9", "replyChannel": "public" },
    });
    crate::low::settle_anchor_failed(&deps, Some(&attributes), "token revoked").await;
    crate::low::settle_anchor_failed(&deps, Some(&attributes), "again").await;
    crate::low::settle_anchor_failed(&deps, Some(&json!({ "replyToCommentId": "c1" })), "x").await;
    let row: (String, String, Option<String>, i64) = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT e."status", e."errorDetail", e."replyText", a."failedCount"
                   FROM "CommentAutomationEvent" e JOIN "CommentAutomation" a ON a."id" = e."automationId""#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(
        row,
        (
            "failed".into(),
            "token revoked".into(),
            Some("Hi".into()),
            1
        )
    );
    crate::low::settle_anchor_delivered(&deps, Some(&attributes)).await;
    let row: (String, i64, i64) = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT e."status", a."failedCount", a."deliveredCount"
                   FROM "CommentAutomationEvent" e JOIN "CommentAutomation" a ON a."id" = e."automationId""#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(row, ("sent".into(), 0, 1));
}

/// A webhook comment queues `processCommentAutomation`; a keyword miss is
/// recorded and nothing is sent.
#[tokio::test]
async fn an_incoming_comment_queues_the_run_and_a_miss_is_recorded() {
    let (mut deps, jobs) = setup().await;
    let channel = RecordingChannel::new();
    deps.channels =
        crate::Channels::builtin().with("messenger", channel.clone() as Arc<dyn Channel>);
    add_automation(
        &deps,
        r#"INSERT INTO "CommentAutomation" ("id", "name", "workspaceId", "type", "post", "publicReply", "includeKeywords")
           VALUES (8, 'B', 2, 'messenger', '{"type":"all","value":[]}', '{"type":"text","value":"Hi"}',
                   '{"type":"contain","value":["zzz"]}')"#,
    )
    .await;
    let graph = Graph(Default::default(), json!({}));
    receive_comment(&deps, &graph, &comment(None), None)
        .await
        .unwrap();
    let queued = jobs.0.lock().unwrap().clone();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].0, PROCESS_COMMENT_AUTOMATION);
    assert_eq!(queued[0].1.job_id.as_deref(), Some("comment-auto-55_9"));

    let run = json!({
        "integrationType": "messenger", "integrationIdentifier": "p1", "workspaceId": "2",
        "conversationId": "0", "contactInboxId": "0", "commentId": "55_9", "postId": "p1_55",
        "message": "How much?", "createdTime": now_ms() / 1000 - 60,
    });
    let (conversation, contact_inbox): (i64, i64) = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "conversationId", "contactInboxId" FROM "Message" WHERE "sourceId" = '55_9'"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap();
    let mut run = run;
    run["conversationId"] = json!(conversation.to_string());
    run["contactInboxId"] = json!(contact_inbox.to_string());
    // Another workspace's id is not run.
    let mut foreign = run.clone();
    foreign["workspaceId"] = json!("3");
    process_comment_automation(&deps, &graph, &foreign)
        .await
        .unwrap();
    process_comment_automation(&deps, &graph, &run)
        .await
        .unwrap();
    assert!(channel.calls().is_empty());
    assert_eq!(jobs.0.lock().unwrap().len(), 1);
    let reason: String = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "reason" FROM "CommentAutomationMiss" WHERE "automationId" = 8"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(reason, "keywordsNotMatched");
}

/// AI moderation: the reply waits for the `commentModeration` job, which
/// classifies through the agent's auto-reply model and hides the comment
/// (log row, miss, hide job); with no agent it resumes the automation.
#[tokio::test]
async fn ai_moderation_holds_the_reply_and_hides_a_flagged_comment() {
    use crate::ai::llm::fake::FakeHttp;
    let (mut deps, jobs) = setup().await;
    let channel = RecordingChannel::new();
    deps.channels =
        crate::Channels::builtin().with("messenger", channel.clone() as Arc<dyn Channel>);
    let http = Arc::new(FakeHttp::new(vec![FakeHttp::openai_text(
        r#"{"flagged":true,"categories":["spam","hate"],"reason":"An ad"}"#,
    )]));
    deps.ai = Arc::new(crate::ai::Ai {
        http: http.clone(),
        embedder: Arc::new(chatbotx_business::ai::knowledge::HttpEmbedder::new()),
    });
    add_automation(
        &deps,
        r#"INSERT INTO "CommentAutomation" ("id", "name", "workspaceId", "type", "post", "publicReply", "hideComments")
           VALUES (9, 'C', 2, 'messenger', '{"type":"all","value":[]}', '{"type":"text","value":"Hi"}',
                   '{"aiModeration":{"enabled":true,"aiAgentId":"90","categories":["spam"],"action":"hide"}}')"#,
    )
    .await;
    let graph = Graph(Default::default(), json!({}));
    replay_missed_comment(&deps, &graph, &comment(Some("9")))
        .await
        .unwrap();
    {
        let queued = jobs.0.lock().unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].0, COMMENT_MODERATION);
        assert!(
            queued[0]
                .1
                .job_id
                .as_deref()
                .unwrap()
                .starts_with("comment-moderation-9-55_9-replay-")
        );
    }
    assert!(channel.calls().is_empty());

    let (message_id, conversation, contact_inbox): (i64, i64, i64) = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "id", "conversationId", "contactInboxId" FROM "Message" WHERE "sourceId" = '55_9'"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await
        .unwrap();
    let data = json!({
        "automationId": "9", "workspaceId": "2", "commentId": "55_9",
        "messageId": message_id.to_string(), "messageCreatedAt": 1,
        "commentAutomation": {
            "integrationType": "messenger", "integrationIdentifier": "p1", "workspaceId": "2",
            "conversationId": conversation.to_string(), "contactInboxId": contact_inbox.to_string(),
            "commentId": "55_9", "postId": "p1_55", "message": "Buy now", "createdTime": 1,
        },
    });
    // No agent yet: fails open and re-runs this automation past moderation.
    comment_moderation(&deps, &data).await.unwrap();
    assert_eq!(
        jobs.0.lock().unwrap()[1].1.job_id.as_deref(),
        Some("comment-auto-55_9-9-moderated")
    );
    add_automation(
        &deps,
        r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (71, 2, 'openaiCompatible');
           INSERT INTO "IntegrationOpenaiCompatible" ("id", "auth", "baseURL", "defaultModel", "enabled", "autoReply", "integrationId", "name", "preset", "workspaceId")
             VALUES (73, NULL, 'http://llm.test/v1', 'local', 1, 1, 71, 'Local', 'lmstudio', 2);
           INSERT INTO "AIAgent" ("id", "workspaceId", "name", "prompt", "messages", "models", "tools", "temperature", "maxOutputTokens")
             VALUES (90, 2, 'Agent', 'x', '[]', '[{"kind":"openaiCompatible","integrationId":"73","model":"local"}]', '[]', 0.5, 300)"#,
    )
    .await;
    comment_moderation(&deps, &data).await.unwrap();
    let request = &http.requests()[0].1;
    assert_eq!(request["temperature"], 0.0);
    assert!(
        request
            .to_string()
            .contains("<<<COMMENT\\nBuy now\\nCOMMENT>>>")
    );
    {
        let queued = jobs.0.lock().unwrap();
        assert_eq!(queued.len(), 3);
        assert_eq!(queued[2].0, crate::chat::CHANGE_CHANNEL_MESSAGE_STATE);
        assert_eq!(
            queued[2].1.job_id.as_deref(),
            Some("moderate-comment-9-55_9")
        );
    }
    let row: (String, String, String, String) = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "source", "categories", "reason", (SELECT "reason" FROM "CommentAutomationMiss" WHERE "automationId" = 9)
                   FROM "CommentModerationLog" WHERE "automationId" = 9"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(
        row,
        (
            "ai".into(),
            r#"["spam"]"#.into(),
            "An ad".into(),
            "moderated".into()
        )
    );
    // The cached verdict is reused; a still-pending hide joins its job id.
    comment_moderation(&deps, &data).await.unwrap();
    assert_eq!(http.requests().len(), 1);
    assert_eq!(
        jobs.0.lock().unwrap()[3].1.job_id.as_deref(),
        Some("moderate-comment-9-55_9")
    );
}

/// `trackCommentTags` (`comment-automation` handler): a tracked comment adds
/// its distinct tags (and the ones not yet known in the inbox) to the
/// commenter's lifetime counters once; a redelivered run counts nothing.
#[tokio::test]
async fn tag_tracking_counts_a_comment_once() {
    let (mut deps, _jobs) = setup().await;
    let channel = RecordingChannel::new();
    deps.channels =
        crate::Channels::builtin().with("messenger", channel.clone() as Arc<dyn Channel>);
    add_automation(
        &deps,
        r#"INSERT INTO "CommentAutomation" ("id", "name", "workspaceId", "type", "post", "options")
           VALUES (7, 'A', 2, 'messenger', '{"type":"all","value":[]}', '{"trackUserTags":true}')"#,
    )
    .await;
    let mut data = comment(Some("7"));
    data["commentData"]["tags"] = json!([{ "id": "u8" }, { "id": "u7" }, { "id": "u8" }]);
    let graph = Graph(Default::default(), json!({}));
    for _ in 0..2 {
        replay_missed_comment(&deps, &graph, &data).await.unwrap();
    }
    assert!(
        graph.0.lock().unwrap().is_empty(),
        "the webhook tags are used"
    );
    let (total, fresh, claimed): (i64, i64, String) = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT k."totalTagged", k."totalNewTagged", m."contentAttributes"
                   FROM "Contact" k JOIN "ContactInbox" ci ON ci."contactId" = k."id"
                   JOIN "Message" m ON m."sourceId" = '55_9'
                   WHERE ci."sourceId" = 'u7'"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await
        .unwrap();
    // u7 (the commenter) is known in the inbox, u8 is not.
    assert_eq!((total, fresh), (2, 1));
    let claimed: Value = serde_json::from_str(&claimed).unwrap();
    assert_eq!(claimed["totalTagged"], 2);
    assert_eq!(claimed["totalNewTagged"], 1);
}

/// Q.P7-V5 comment extras: a Messenger comment on a live video is marked
/// live (and the post remembered), and its post is tracked with the Graph
/// details and linked to the commenter's contact inbox.
#[tokio::test]
async fn a_messenger_comment_tracks_its_post_and_detects_a_live_video() {
    let (deps, _) = setup().await;
    let graph = Graph(
        Default::default(),
        json!({ "live_broadcast_timestamp": 1_700_000_000, "message": "My post" }),
    );
    receive_comment(&deps, &graph, &comment(None), None)
        .await
        .unwrap();
    let calls = graph.0.lock().unwrap().clone();
    assert!(
        calls.iter().any(|c| c.contains("/55_9?fields=attachment")),
        "{calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.contains("/p1_55?fields=message")),
        "{calls:?}"
    );
    let (caption, links, attributes): (String, i64, String) = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT cp."caption",
                          (SELECT COUNT(*) FROM "ContactInboxPost" l WHERE l."postId" = cp."id"),
                          (SELECT "contentAttributes" FROM "Message" WHERE "sourceId" = '55_9')
                   FROM "ChannelPost" cp WHERE cp."externalPostId" = 'p1_55' AND cp."channel" = 'messenger'"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(caption, "My post");
    assert_eq!(links, 1);
    assert!(
        attributes.contains(r#""isLiveComment":true"#),
        "{attributes}"
    );
    let store = chatbotx_kv::DurableKv::new(deps.db.clone(), Arc::new(chatbotx_kv::SystemClock));
    assert_eq!(
        store
            .get::<bool>("comment-live-post:5:p1_55")
            .await
            .unwrap(),
        Some(true)
    );
}
