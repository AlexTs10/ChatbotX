//! Ported step tests (Q.W3). The TypeScript files mock the services; here
//! the handlers run through `runFlowNode` on a migrated SQLite file and the
//! assertions read the rows and the recorded jobs. One module per TS file.

use std::sync::Arc;

use serde_json::{Value, json};

use super::test_support::*;
use crate::variables::{DbSources, VariableDeps};

pub(crate) fn with_variables(fx: &mut Fixture) {
    fx.deps.variables = Some(Arc::new(VariableDeps {
        sources: DbSources {
            db: fx.deps.db.clone(),
            app_url: "http://app.test".into(),
        },
        cache: chatbotx_kv::cache::Cache::new(Arc::new(chatbotx_kv::SystemClock)),
        encryption: chatbotx_core::encryption::Encryption::new(
            chatbotx_core::encryption::Keys::new(&"ab".repeat(32)).unwrap(),
        ),
    }));
}

/// `condition-handler.test.ts`.
mod condition_handler {
    use super::*;

    fn step(first: Value, second: Value) -> Value {
        json!({
            "id": "1", "stepType": "condition", "otherwiseId": "otherwise-handle",
            "cases": [
                { "id": "empty-case", "operator": "and", "conditions": [] },
                { "id": "first-case", "operator": "and", "conditions": [first] },
                { "id": "second-case", "operator": "or", "conditions": [second] },
            ],
        })
    }

    fn name_is(value: &str) -> Value {
        json!({ "field": "fullName", "operator": "contains", "value": value })
    }

    fn edges() -> Value {
        json!([
            { "sourceHandle": "first-case", "target": "first-target" },
            { "sourceHandle": "second-case", "target": "second-target" },
            { "sourceHandle": "otherwise-handle", "target": "otherwise-target" },
        ])
    }

    #[tokio::test]
    async fn skips_empty_cases_and_routes_to_the_first_matching_case() {
        let fx = Fixture::new().await;
        fx.node(json!([step(name_is("Bob"), name_is("Ada"))]), edges())
            .await;
        fx.run(json!({})).await.unwrap();
        let jobs = fx.jobs.send_flows();
        assert_eq!(jobs.len(), 1);
        let job = &jobs[0];
        assert_eq!(job["conversationId"], "6");
        assert_eq!(job["contactInboxId"], "5");
        assert_eq!(job["flowId"], "10");
        assert_eq!(job["flowVersionId"], "11");
        assert_eq!(job["nodeId"], "second-target");
        assert_eq!(job["origin"], "webhook");
    }

    #[tokio::test]
    async fn matched_and_otherwise_branches_forward_started_by_contact_message() {
        for (matched, flag) in [
            (true, Some(true)),
            (true, None),
            (false, Some(true)),
            (false, None),
        ] {
            let fx = Fixture::new().await;
            let first = name_is(if matched { "Ada" } else { "Bob" });
            fx.node(json!([step(first, name_is("Bob"))]), edges()).await;
            let extra = flag.map_or(json!({}), |f| json!({ "startedByContactMessage": f }));
            fx.run(extra).await.unwrap();
            let job = &fx.jobs.send_flows()[0];
            assert_eq!(
                job["nodeId"],
                if matched {
                    "first-target"
                } else {
                    "otherwise-target"
                }
            );
            assert_eq!(
                job.get("startedByContactMessage").and_then(Value::as_bool),
                flag
            );
        }
    }

    #[tokio::test]
    async fn routes_to_otherwise_when_no_case_matches() {
        let fx = Fixture::new().await;
        fx.node(json!([step(name_is("Bob"), name_is("Eve"))]), edges())
            .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.jobs.send_flows()[0]["nodeId"], "otherwise-target");
    }

    #[tokio::test]
    async fn routes_to_otherwise_when_filter_evaluation_throws() {
        let fx = Fixture::new().await;
        let mut s = step(name_is("Ada"), name_is("Ada"));
        // An operator that does not parse fails the evaluation.
        s["cases"][1]["operator"] = json!(5);
        fx.node(json!([s]), edges()).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.jobs.send_flows()[0]["nodeId"], "otherwise-target");
    }

    #[tokio::test]
    async fn does_not_enqueue_when_the_matched_handle_has_no_connected_edge() {
        let fx = Fixture::new().await;
        fx.node(json!([step(name_is("Ada"), name_is("Ada"))]), json!([]))
            .await;
        fx.run(json!({})).await.unwrap();
        assert!(fx.jobs.send_flows().is_empty());
    }

    #[tokio::test]
    async fn passes_variable_resolved_case_values_to_contact_filter_matching() {
        let mut fx = Fixture::new().await;
        with_variables(&mut fx);
        fx.node(
            json!([step(name_is("{{first_name}}"), name_is("Bob"))]),
            edges(),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.jobs.send_flows()[0]["nodeId"], "first-target");
    }

    #[tokio::test]
    async fn threads_appointment_id_through_the_matched_branch() {
        let fx = Fixture::new().await;
        fx.node(json!([step(name_is("Ada"), name_is("Bob"))]), edges())
            .await;
        fx.run(json!({ "appointmentId": "77" })).await.unwrap();
        let job = &fx.jobs.send_flows()[0];
        assert_eq!(job["nodeId"], "first-target");
        assert_eq!(job["appointmentId"], "77");
    }
}

/// `contact-tag-steps.test.ts` (the event-scope cases are n-a: the tag /
/// unsubscribe events are Q.W2).
mod contact_tag_steps {
    use super::*;

    async fn sequence(fx: &Fixture) {
        fx.exec(r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (20, 'S', 2);"#)
            .await;
    }

    const ENROLLED: &str =
        r#"SELECT count(*) FROM "ContactOnSequence" WHERE "contactId" = 3 AND "sequenceId" = 20"#;

    #[tokio::test]
    async fn unsubscribe_sequence_removes_the_enrollment() {
        let fx = Fixture::new().await;
        sequence(&fx).await;
        fx.node(
            json!([{ "id": "1", "stepType": "subscribeSequence", "sequenceId": "20" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.int(ENROLLED).await, Some(1));
        fx.node(
            json!([{ "id": "1", "stepType": "unsubscribeSequence", "sequenceId": "20" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.int(ENROLLED).await, Some(0));
    }

    #[tokio::test]
    async fn steps_without_a_sequence_id_do_nothing() {
        let fx = Fixture::new().await;
        sequence(&fx).await;
        for step_type in ["subscribeSequence", "unsubscribeSequence"] {
            fx.node(json!([{ "id": "1", "stepType": step_type }]), json!([]))
                .await;
            fx.run(json!({})).await.unwrap();
        }
        assert_eq!(fx.int(ENROLLED).await, Some(0));
    }

    #[tokio::test]
    async fn subscribe_and_unsubscribe_broadcast_set_the_subscription_time() {
        let fx = Fixture::new().await;
        const AT: &str = r#"SELECT "broadcastSubscribedAt" FROM "Contact" WHERE "id" = 3"#;
        fx.node(
            json!([{ "id": "1", "stepType": "subscribeBroadcast" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        let first = fx.int(AT).await.expect("subscribed");
        fx.run(json!({})).await.unwrap();
        assert_eq!(
            fx.int(AT).await,
            Some(first),
            "a second subscribe keeps the first time"
        );
        fx.node(
            json!([{ "id": "1", "stepType": "unsubscribeBroadcast" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.int(AT).await, None);
    }

    #[tokio::test]
    async fn add_and_remove_contact_tag_use_the_conversation_contact() {
        let fx = Fixture::new().await;
        const LINKED: &str = r#"SELECT count(*) FROM "ContactToTag" ct JOIN "Tag" t ON t."id" = ct."tagId" WHERE ct."contactId" = 3 AND t."name" = 'vip' AND t."workspaceId" = 2"#;
        fx.node(
            json!([{ "id": "1", "stepType": "addContactTag", "tags": ["vip"] }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.int(LINKED).await, Some(1));
        fx.node(
            json!([{ "id": "1", "stepType": "removeContactTag", "tags": ["vip"] }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.int(LINKED).await, Some(0));
    }
}

/// `set-custom-field-step.test.ts` and `clear-custom-field-step.test.ts`.
mod custom_field_steps {
    use super::*;

    const VALUE: &str = r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = 3 AND "customFieldId" = 30"#;

    async fn field(fx: &Fixture, kind: &'static str) {
        let sql = match kind {
            "number" => {
                r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (30, 'score', 'number', 2);"#
            }
            "datetime" => {
                r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (30, 'when', 'datetime', 2);"#
            }
            _ => {
                r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (30, 'plan', 'shortText', 2);"#
            }
        };
        fx.exec(sql).await;
    }

    fn set(value: &str) -> Value {
        json!([{ "id": "1", "stepType": "setCustomField", "inputFieldId": "30", "operation": "O01", "value": value }])
    }

    #[tokio::test]
    async fn resolves_variable_tokens_in_the_value_before_persisting() {
        let mut fx = Fixture::new().await;
        with_variables(&mut fx);
        field(&fx, "text").await;
        fx.node(set("Hi {{first_name}}"), json!([])).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.text(VALUE).await.as_deref(), Some("Hi Ada"));
    }

    #[tokio::test]
    async fn a_plain_value_is_stored_as_is() {
        let fx = Fixture::new().await;
        field(&fx, "text").await;
        fx.node(set("gold"), json!([])).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.text(VALUE).await.as_deref(), Some("gold"));
    }

    #[tokio::test]
    async fn a_blank_value_on_a_datetime_field_stamps_now() {
        let fx = Fixture::new().await;
        field(&fx, "datetime").await;
        fx.node(set(""), json!([])).await;
        fx.run(json!({})).await.unwrap();
        assert!(fx.text(VALUE).await.is_some_and(|v| v.starts_with("20")));
    }

    #[tokio::test]
    async fn a_failing_write_is_swallowed_and_the_next_step_still_runs() {
        let fx = Fixture::new().await;
        field(&fx, "number").await;
        fx.node(
            json!([
                { "id": "1", "stepType": "setCustomField", "inputFieldId": "30", "operation": "O01", "value": "not a number" },
                { "id": "2", "stepType": "addContactNotes", "content": "after" },
            ]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.text(VALUE).await, None);
        let next = &fx.jobs.send_flows()[0];
        assert_eq!(
            next["startFromStepId"], "2",
            "the node goes on with the next step"
        );
    }

    #[tokio::test]
    async fn a_calculator_rejection_is_logged_and_the_flow_keeps_going() {
        let fx = Fixture::new().await;
        field(&fx, "text").await;
        // Increase on a text field is rejected by the calculator.
        fx.node(
            json!([{ "id": "1", "stepType": "setCustomField", "inputFieldId": "30", "operation": "O04", "value": "1" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.text(VALUE).await, None);
    }

    #[tokio::test]
    async fn clear_custom_field_deletes_the_contact_value() {
        let fx = Fixture::new().await;
        field(&fx, "text").await;
        fx.exec(r#"INSERT INTO "ContactCustomField" ("id", "value", "contactId", "customFieldId") VALUES (31, 'gold', 3, 30);"#).await;
        fx.node(
            json!([{ "id": "1", "stepType": "clearCustomField", "inputFieldId": "30" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.text(VALUE).await, None);
    }
}

/// Contact field steps of `contact.ts` without a TS test file of their own.
mod contact_steps {
    use super::*;

    #[tokio::test]
    async fn email_flags_notes_block_and_delete() {
        let fx = Fixture::new().await;
        for (step_type, sql, want) in [
            (
                "markEmailVerified",
                r#"SELECT "emailVerified" FROM "Contact" WHERE "id" = 3"#,
                Some(1),
            ),
            (
                "optInEmail",
                r#"SELECT "emailOptIn" FROM "Contact" WHERE "id" = 3"#,
                Some(1),
            ),
            (
                "optOutEmail",
                r#"SELECT "emailOptIn" FROM "Contact" WHERE "id" = 3"#,
                Some(0),
            ),
            (
                "blockContact",
                r#"SELECT "blockedAt" IS NOT NULL FROM "Contact" WHERE "id" = 3"#,
                Some(1),
            ),
        ] {
            fx.node(json!([{ "id": "1", "stepType": step_type }]), json!([]))
                .await;
            fx.run(json!({})).await.unwrap();
            assert_eq!(fx.int(sql).await, want, "{step_type}");
        }
        fx.node(
            json!([{ "id": "1", "stepType": "addContactNotes", "content": "hello" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(
            fx.text(r#"SELECT "text" FROM "ContactNote" WHERE "contactId" = 3"#)
                .await
                .as_deref(),
            Some("hello")
        );
        fx.node(
            json!([{ "id": "1", "stepType": "deleteContact" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(
            fx.int(r#"SELECT count(*) FROM "Contact" WHERE "id" = 3"#)
                .await,
            Some(0)
        );
    }
}

/// `auto-assign-step-handler.test.ts`: users 7 / 8 are members; a member
/// with a schedule and no interval is off shift, one without is on shift.
mod auto_assign_step_handler {
    use super::*;

    async fn agents(fx: &Fixture) {
        fx.exec(
            r#"INSERT INTO "User" ("id", "email") VALUES (7, 'a@x.test'), (8, 'b@x.test');
               INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role") VALUES (71, 2, 7, 'agent'), (81, 2, 8, 'agent');
               INSERT INTO "InboxTeam" ("id", "workspaceId", "name") VALUES (40, 2, 'Team');
               INSERT INTO "WorkingHours" ("id", "workspaceId", "name") VALUES (50, 2, 'Never');"#,
        )
        .await;
    }

    async fn off_shift(fx: &Fixture, user: i64) {
        fx.sql(move |tx| {
            tx.execute(
                r#"INSERT INTO "WorkingHoursMember" ("id", "workingHoursId", "workspaceId", "userId") VALUES (?1, 50, 2, ?2)"#,
                chatbotx_db::rusqlite::params![user + 500, user],
            )?;
            Ok(())
        })
        .await;
    }

    /// Two open conversations for user 7 (load 2), none for user 8.
    async fn load(fx: &Fixture) {
        fx.exec(
            r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (90, 2), (91, 2);
               INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "assignedUserId") VALUES (92, 2, 90, 7), (93, 2, 91, 7);"#,
        )
        .await;
    }

    async fn run(fx: &Fixture, ids: Value) -> (Option<i64>, Option<i64>) {
        fx.node(json!([{ "id": "1", "stepType": "autoAssignConversation", "assignedIds": ids, "rule": "ALL_TIME" }]), json!([]))
            .await;
        fx.run(json!({})).await.unwrap();
        (
            fx.int(r#"SELECT "assignedUserId" FROM "Conversation" WHERE "id" = 6"#)
                .await,
            fx.int(r#"SELECT "assignedInboxTeamId" FROM "Conversation" WHERE "id" = 6"#)
                .await,
        )
    }

    #[tokio::test]
    async fn picks_the_least_loaded_agent_among_those_on_shift() {
        let fx = Fixture::new().await;
        agents(&fx).await;
        load(&fx).await;
        assert_eq!(run(&fx, json!(["u_7", "u_8"])).await, (Some(8), None));
        assert_eq!(fx.jobs.named("notifyConversationAssigned").len(), 1);
    }

    #[tokio::test]
    async fn an_off_shift_agent_is_skipped_even_when_least_loaded() {
        let fx = Fixture::new().await;
        agents(&fx).await;
        load(&fx).await;
        off_shift(&fx, 8).await;
        assert_eq!(run(&fx, json!(["u_7", "u_8"])).await, (Some(7), None));
    }

    #[tokio::test]
    async fn falls_back_to_every_candidate_when_nobody_is_on_shift() {
        let fx = Fixture::new().await;
        agents(&fx).await;
        load(&fx).await;
        off_shift(&fx, 7).await;
        off_shift(&fx, 8).await;
        assert_eq!(run(&fx, json!(["u_7", "u_8"])).await, (Some(8), None));
    }

    #[tokio::test]
    async fn teams_are_never_filtered_for_team_only_steps() {
        let fx = Fixture::new().await;
        agents(&fx).await;
        assert_eq!(run(&fx, json!(["t_40"])).await, (None, Some(40)));
    }

    #[tokio::test]
    async fn a_candidate_team_stays_available_so_off_shift_users_are_dropped() {
        let fx = Fixture::new().await;
        agents(&fx).await;
        off_shift(&fx, 7).await;
        assert_eq!(run(&fx, json!(["u_7", "t_40"])).await, (None, Some(40)));
    }

    #[tokio::test]
    async fn a_tie_on_load_still_picks_a_user_before_a_team() {
        let fx = Fixture::new().await;
        agents(&fx).await;
        assert_eq!(run(&fx, json!(["u_7", "t_40"])).await, (Some(7), None));
    }

    #[tokio::test]
    async fn no_assignee_or_no_eligible_agent_follows_the_error_state() {
        let fx = Fixture::new().await;
        for ids in [json!([]), json!(["u_99"])] {
            fx.node(
                json!([{ "id": "1", "stepType": "autoAssignConversation", "assignedIds": ids, "rule": "ALL_TIME",
                         "states": [{ "id": "err", "stateType": "error" }] }]),
                json!([{ "sourceHandle": "err", "target": "n2" }]),
            )
            .await;
            fx.run(json!({})).await.unwrap();
        }
        let routed: Vec<_> = fx
            .jobs
            .send_flows()
            .iter()
            .map(|j| j["nodeId"].clone())
            .collect();
        assert_eq!(routed, vec![json!("n2"), json!("n2")]);
    }
}

/// `mark-conversation-read-state-steps.test.ts`.
mod mark_conversation_read_state_steps {
    use super::*;
    use crate::channel::RecordingChannel;

    async fn messenger(fx: &mut Fixture, channel: &'static str) -> Arc<RecordingChannel> {
        fx.sql(move |tx| {
            tx.execute(r#"UPDATE "ContactInbox" SET "channel" = ?1 WHERE "id" = 5"#, chatbotx_db::rusqlite::params![channel])?;
            let table = if channel == "whatsapp" { "IntegrationWhatsapp" } else { "IntegrationMessenger" };
            if table == "IntegrationMessenger" {
                tx.execute(
                    r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId")
                       VALUES (60, '{}', 'p', 'Page', '[]', '[]', '[]', 2, 4)"#,
                    [],
                )?;
            }
            Ok(())
        })
        .await;
        let channel_double = RecordingChannel::new();
        fx.deps.channels = crate::Channels::default().with(channel, channel_double.clone());
        channel_double
    }

    #[tokio::test]
    async fn mark_unread_clears_the_read_marker() {
        let fx = Fixture::new().await;
        fx.exec(r#"UPDATE "Conversation" SET "agentLastReadAt" = 5 WHERE "id" = 6;"#)
            .await;
        fx.node(
            json!([{ "id": "1", "stepType": "markConversationAsUnread" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(
            fx.int(r#"SELECT "agentLastReadAt" FROM "Conversation" WHERE "id" = 6"#)
                .await,
            None
        );
    }

    #[tokio::test]
    async fn marks_inbox_state_read_then_sends_mark_seen_on_messenger() {
        let mut fx = Fixture::new().await;
        let channel = messenger(&mut fx, "messenger").await;
        fx.node(
            json!([{ "id": "1", "stepType": "markConversationAsRead" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert!(
            fx.int(r#"SELECT "agentLastReadAt" FROM "Conversation" WHERE "id" = 6"#)
                .await
                .is_some()
        );
        let calls = channel.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].handler, "agentMarkAsRead");
        assert_eq!(calls[0].data["messageSourceId"], Value::Null);
    }

    #[tokio::test]
    async fn a_comment_thread_conversation_never_sends_a_dm_receipt() {
        let mut fx = Fixture::new().await;
        let channel = messenger(&mut fx, "messenger").await;
        fx.exec(r#"UPDATE "Conversation" SET "sourceId" = 'post-1' WHERE "id" = 6;"#)
            .await;
        fx.node(
            json!([{ "id": "1", "stepType": "markConversationAsRead" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert!(channel.calls().is_empty());
    }

    #[tokio::test]
    async fn whatsapp_with_no_incoming_message_skips_the_receipt() {
        let mut fx = Fixture::new().await;
        let channel = messenger(&mut fx, "whatsapp").await;
        fx.node(
            json!([{ "id": "1", "stepType": "markConversationAsRead" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert!(channel.calls().is_empty());
    }

    #[tokio::test]
    async fn a_channel_error_is_logged_and_does_not_throw() {
        // No integration row for the webchat inbox: the receipt is skipped
        // (webchat has no receipt) and a Messenger lookup failure is swallowed.
        let mut fx = Fixture::new().await;
        fx.exec(r#"UPDATE "ContactInbox" SET "channel" = 'instagram' WHERE "id" = 5;"#)
            .await;
        fx.deps.channels = crate::Channels::default().with("instagram", RecordingChannel::new());
        fx.node(
            json!([{ "id": "1", "stepType": "markConversationAsRead" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
    }

    #[tokio::test]
    async fn disable_bot_opts_into_reopening_the_window_for_a_message_driven_run() {
        let fx = Fixture::new().await;
        fx.exec(r#"UPDATE "Conversation" SET "contactRepliedAt" = 1000 WHERE "id" = 6;"#)
            .await;
        fx.node(json!([{ "id": "1", "stepType": "disableBot" }]), json!([]))
            .await;
        fx.run(json!({ "startedByContactMessage": true }))
            .await
            .unwrap();
        assert_eq!(
            fx.int(r#"SELECT "botEnabled" FROM "Conversation" WHERE "id" = 6"#)
                .await,
            Some(0)
        );
        assert_eq!(
            fx.int(r#"SELECT "awaitingHumanReplySince" FROM "Conversation" WHERE "id" = 6"#)
                .await,
            Some(1000)
        );
    }

    #[tokio::test]
    async fn disable_bot_of_a_broadcast_run_keeps_the_window_closed() {
        let fx = Fixture::new().await;
        fx.exec(r#"UPDATE "Conversation" SET "contactRepliedAt" = 1000 WHERE "id" = 6;"#)
            .await;
        fx.node(json!([{ "id": "1", "stepType": "disableBot" }]), json!([]))
            .await;
        fx.run(json!({ "startedByContactMessage": true, "isBulkBroadcast": true }))
            .await
            .unwrap();
        assert_eq!(
            fx.int(r#"SELECT "awaitingHumanReplySince" FROM "Conversation" WHERE "id" = 6"#)
                .await,
            None
        );
    }
}

/// Conversation steps of `step-handlers.ts` without a TS test file.
mod conversation_steps {
    use super::*;

    #[tokio::test]
    async fn archive_follow_bot_and_assign_steps_write_the_conversation() {
        let fx = Fixture::new().await;
        fx.exec(
            r#"INSERT INTO "User" ("id", "email") VALUES (7, 'a@x.test');
               INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role") VALUES (71, 2, 7, 'agent');"#,
        )
        .await;
        for (step, sql, want) in [
            (
                json!({ "stepType": "archiveConversation" }),
                r#"SELECT "archivedAt" IS NOT NULL FROM "Conversation" WHERE "id" = 6"#,
                Some(1),
            ),
            (
                json!({ "stepType": "unarchiveConversation" }),
                r#"SELECT "archivedAt" IS NOT NULL FROM "Conversation" WHERE "id" = 6"#,
                Some(0),
            ),
            (
                json!({ "stepType": "followConversation" }),
                r#"SELECT "followed" FROM "Conversation" WHERE "id" = 6"#,
                Some(1),
            ),
            (
                json!({ "stepType": "unfollowConversation" }),
                r#"SELECT "followed" FROM "Conversation" WHERE "id" = 6"#,
                Some(0),
            ),
            (
                json!({ "stepType": "disableBot" }),
                r#"SELECT "botEnabled" FROM "Conversation" WHERE "id" = 6"#,
                Some(0),
            ),
            (
                json!({ "stepType": "enableBot" }),
                r#"SELECT "botEnabled" FROM "Conversation" WHERE "id" = 6"#,
                Some(1),
            ),
            (
                json!({ "stepType": "assignConversation", "assignedId": "u_7" }),
                r#"SELECT "assignedUserId" FROM "Conversation" WHERE "id" = 6"#,
                Some(7),
            ),
            (
                json!({ "stepType": "assignConversation", "assignedId": "u_99" }),
                r#"SELECT "assignedUserId" FROM "Conversation" WHERE "id" = 6"#,
                Some(7),
            ),
            (
                json!({ "stepType": "unassignConversation" }),
                r#"SELECT "assignedUserId" FROM "Conversation" WHERE "id" = 6"#,
                None,
            ),
        ] {
            let mut step = step;
            step["id"] = json!("1");
            fx.node(json!([step.clone()]), json!([])).await;
            fx.run(json!({})).await.unwrap();
            assert_eq!(fx.int(sql).await, want, "{step}");
        }
    }
}

/// `thread-control-step.test.ts`: every channel answers "not supported"
/// until the thread-control unit (Q.D12), so the cases that reach the
/// channel are n-a; the routing on failure is ported.
mod thread_control_step {
    use super::*;

    #[tokio::test]
    async fn an_unsupported_channel_follows_the_error_path() {
        let fx = Fixture::new().await;
        fx.node(
            json!([{ "id": "1", "stepType": "threadControl", "action": "pass", "targetRole": "primary", "metadata": "note",
                     "states": [{ "id": "ok", "stateType": "success" }, { "id": "err", "stateType": "error" }] }]),
            json!([{ "sourceHandle": "ok", "target": "n2" }, { "sourceHandle": "err", "target": "n3" }]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.jobs.send_flows()[0]["nodeId"], "n3");
    }
}

/// `follow-up-step.test.ts`, `resume-follow-up.test.ts`, `resume-wait.test.ts`
/// and `smart-delay-resume-job.test.ts`.
mod smart_delay_steps {
    use super::*;

    const ROW: &str = r#"SELECT count(*) FROM "ContactOnSmartDelay" WHERE "type" = 'followUp' AND "nodeId" = 'n2' AND "stepId" = '1' AND "flowVersionId" = 11 AND "conversationId" = 6 AND "contactInboxId" = 5"#;

    async fn follow_up(fx: &Fixture, duration: i64, unit: &str, edges: Value) {
        fx.node(
            json!([{ "id": "1", "stepType": "followUp", "duration": duration, "unit": unit }]),
            edges,
        )
        .await;
    }

    fn to_n2() -> Value {
        json!([{ "sourceHandle": "n1", "target": "n2" }])
    }

    #[tokio::test]
    async fn upserts_a_follow_up_row_and_enqueues_resume_follow_up_for_short_delays() {
        let fx = Fixture::new().await;
        follow_up(&fx, 1, "minutes", to_n2()).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.int(ROW).await, Some(1));
        assert_eq!(
            fx.text(r#"SELECT "status" FROM "ContactOnSmartDelay""#)
                .await
                .as_deref(),
            Some("scheduled")
        );
        let jobs = fx.jobs.named("resumeFollowUp");
        assert_eq!(jobs.len(), 1);
        let id = fx
            .int(r#"SELECT "id" FROM "ContactOnSmartDelay""#)
            .await
            .unwrap();
        let trigger = fx
            .int(r#"SELECT "triggerAt" FROM "ContactOnSmartDelay""#)
            .await
            .unwrap();
        assert_eq!(
            jobs[0].data,
            json!({ "type": "resumeFollowUp", "data": { "smartDelayId": id.to_string() } })
        );
        assert_eq!(
            jobs[0].opts.job_id,
            Some(format!("smart-delay-{id}-{trigger}"))
        );
        assert!(jobs[0].opts.delay_ms.is_some_and(|d| d <= 60_000));
        // The wait stops the node: no continuation, no edge.
        assert!(fx.jobs.send_flows().is_empty());
    }

    #[tokio::test]
    async fn keeps_long_delay_rows_pending_for_the_scanner() {
        let fx = Fixture::new().await;
        follow_up(&fx, 10, "minutes", to_n2()).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(
            fx.text(r#"SELECT "status" FROM "ContactOnSmartDelay""#)
                .await
                .as_deref(),
            Some("pending")
        );
        assert!(fx.jobs.named("resumeFollowUp").is_empty());
    }

    #[tokio::test]
    async fn scheduling_the_same_follow_up_twice_reuses_the_upsert_path() {
        let fx = Fixture::new().await;
        follow_up(&fx, 10, "minutes", to_n2()).await;
        fx.run(json!({})).await.unwrap();
        fx.run(json!({})).await.unwrap();
        assert_eq!(
            fx.int(r#"SELECT count(*) FROM "ContactOnSmartDelay""#)
                .await,
            Some(1)
        );
    }

    #[tokio::test]
    async fn skips_without_creating_a_row_when_no_node_is_connected() {
        let fx = Fixture::new().await;
        follow_up(&fx, 1, "minutes", json!([])).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(
            fx.int(r#"SELECT count(*) FROM "ContactOnSmartDelay""#)
                .await,
            Some(0)
        );
    }

    #[tokio::test]
    async fn leaves_the_row_pending_when_immediate_enqueue_fails() {
        let fx = Fixture::new().await;
        follow_up(&fx, 1, "minutes", to_n2()).await;
        fx.jobs.fail_next();
        fx.run(json!({})).await.unwrap();
        assert_eq!(
            fx.text(r#"SELECT "status" FROM "ContactOnSmartDelay""#)
                .await
                .as_deref(),
            Some("pending")
        );
    }

    async fn due(fx: &Fixture, kind: &'static str, metadata: Option<&'static str>) -> String {
        fx.sql(move |tx| {
            tx.execute(
                r#"INSERT INTO "ContactOnSmartDelay" ("id", "workspaceId", "flowId", "flowVersionId", "contactInboxId", "conversationId", "nodeId", "stepId", "metadata", "type", "createdAt", "triggerAt", "status")
                   VALUES (70, 2, 10, 11, 5, 6, 'n2', '1', ?1, ?2, 100, 200, 'scheduled')"#,
                chatbotx_db::rusqlite::params![metadata, kind],
            )?;
            Ok(())
        })
        .await;
        "70".into()
    }

    async fn status(fx: &Fixture) -> Option<String> {
        fx.text(r#"SELECT "status" FROM "ContactOnSmartDelay" WHERE "id" = 70"#)
            .await
    }

    #[tokio::test]
    async fn follow_up_resume_cancels_without_sending_when_the_contact_replied() {
        let fx = Fixture::new().await;
        let id = due(&fx, "followUp", None).await;
        fx.exec(r#"UPDATE "ContactInbox" SET "lastIncomingMessageAt" = 150 WHERE "id" = 5;"#)
            .await;
        super::super::delay::run_follow_up_resume(&fx.deps, &json!({ "smartDelayId": id }))
            .await
            .unwrap();
        assert_eq!(status(&fx).await.as_deref(), Some("canceled"));
        assert!(fx.jobs.send_flows().is_empty());
    }

    #[tokio::test]
    async fn follow_up_resume_continues_the_flow_when_the_contact_stayed_silent() {
        let fx = Fixture::new().await;
        let id = due(
            &fx,
            "followUp",
            Some(r#"{"type":"sequenceSchedule","contactInboxId":"5","sequenceStepId":"9","sequenceId":"8","dispatchId":"7"}"#),
        )
        .await;
        super::super::delay::run_follow_up_resume(&fx.deps, &json!({ "smartDelayId": id }))
            .await
            .unwrap();
        assert_eq!(status(&fx).await.as_deref(), Some("completed"));
        let jobs = fx.jobs.send_flows();
        assert_eq!(
            jobs,
            vec![
                json!({ "conversationId": "6", "contactInboxId": "5", "flowId": "10", "flowVersionId": "11", "nodeId": "n2",
                         "metadata": { "type": "sequenceSchedule", "contactInboxId": "5", "sequenceStepId": "9", "sequenceId": "8", "dispatchId": "7" } })
            ]
        );
        assert!(
            !jobs[0]
                .as_object()
                .unwrap()
                .contains_key("startedByContactMessage")
        );
    }

    #[tokio::test]
    async fn resume_does_nothing_when_another_worker_claimed_the_row_or_it_is_not_due() {
        let fx = Fixture::new().await;
        let id = due(&fx, "followUp", None).await;
        fx.exec(r#"UPDATE "ContactOnSmartDelay" SET "status" = 'completed' WHERE "id" = 70;"#)
            .await;
        super::super::delay::run_follow_up_resume(&fx.deps, &json!({ "smartDelayId": id.clone() }))
            .await
            .unwrap();
        fx.exec(r#"UPDATE "ContactOnSmartDelay" SET "status" = 'scheduled', "triggerAt" = 99999999999999 WHERE "id" = 70;"#).await;
        super::super::delay::run_follow_up_resume(&fx.deps, &json!({ "smartDelayId": id.clone() }))
            .await
            .unwrap();
        super::super::delay::run_wait_resume(
            &fx.deps,
            &json!({ "smartDelayId": id }),
            "k:1".into(),
        )
        .await
        .unwrap();
        assert!(fx.jobs.all().is_empty());
        assert_eq!(status(&fx).await.as_deref(), Some("scheduled"));
    }

    #[tokio::test]
    async fn wait_resume_runs_the_connected_node_after_claiming_the_row() {
        let fx = Fixture::new().await;
        fx.flow(
            json!([{ "id": "n2", "data": { "details": { "steps": [{ "id": "5", "stepType": "addContactNotes", "content": "resumed" }] } } }]),
            json!([]),
        )
        .await;
        let id = due(&fx, "waitNode", None).await;
        super::super::delay::run_wait_resume(
            &fx.deps,
            &json!({ "smartDelayId": id }),
            "job-9:1".into(),
        )
        .await
        .unwrap();
        assert_eq!(status(&fx).await.as_deref(), Some("completed"));
        assert_eq!(
            fx.text(r#"SELECT "text" FROM "ContactNote" WHERE "contactId" = 3"#)
                .await
                .as_deref(),
            Some("resumed")
        );
    }

    #[tokio::test]
    async fn wait_resume_requeues_the_claimed_row_and_rethrows_when_the_flow_fails() {
        let fx = Fixture::new().await;
        // No flow version 11: the resumed run fails.
        let id = due(&fx, "waitNode", None).await;
        let result = super::super::delay::run_wait_resume(
            &fx.deps,
            &json!({ "smartDelayId": id }),
            "k:1".into(),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(status(&fx).await.as_deref(), Some("scheduled"));
    }

    #[tokio::test]
    async fn the_wait_step_schedules_a_wait_node_row() {
        let fx = Fixture::new().await;
        fx.node(
            json!([{ "id": "1", "stepType": "wait", "delayType": "duration", "duration": 30, "unit": "seconds" }]),
            to_n2(),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(
            fx.text(r#"SELECT "type" FROM "ContactOnSmartDelay""#)
                .await
                .as_deref(),
            Some("waitNode")
        );
        assert_eq!(fx.jobs.named("resumeWait").len(), 1);
    }

    #[tokio::test]
    async fn a_wait_without_a_trigger_time_follows_the_error_state() {
        let fx = Fixture::new().await;
        fx.node(
            json!([{ "id": "1", "stepType": "wait", "delayType": "date", "dateType": "specific", "datetime": "",
                     "states": [{ "id": "err", "stateType": "error" }] }]),
            json!([{ "sourceHandle": "err", "target": "n3" }, { "sourceHandle": "n1", "target": "n2" }]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(fx.jobs.send_flows()[0]["nodeId"], "n3");
    }
}

/// `start-external-step-handlers.test.ts` (Q.W1 handlers) and the split
/// traffic step.
mod start_and_split_steps {
    use super::*;

    #[tokio::test]
    async fn start_external_flow_forwards_carried_fields_and_adds_no_node_id() {
        let fx = Fixture::new().await;
        fx.node(
            json!([{ "id": "1", "stepType": "startExternalFlow", "flowId": "44" }]),
            json!([]),
        )
        .await;
        fx.run(json!({ "metadata": { "type": "sequenceSchedule", "contactInboxId": "5", "sequenceStepId": "9", "sequenceId": "8", "dispatchId": "7" }, "sendFrom": "inbox", "sentByUserId": "1",
                       "triggerMessageId": "m1", "startedByContactMessage": true, "appointmentId": "77" }))
            .await
            .unwrap();
        // `send-flow-job.test.ts`: the core, the origin and the carried
        // fields only (no node id, no flow version of the running flow).
        let job = fx.jobs.send_flows().remove(0);
        let mut keys: Vec<&str> = job
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "appointmentId",
                "contactInboxId",
                "conversationId",
                "flowId",
                "metadata",
                "nodeVisits",
                "origin",
                "sendFrom",
                "sentByUserId",
                "startedByContactMessage",
                "triggerMessageId"
            ]
        );
        assert_eq!(
            (job["flowId"].clone(), job["origin"].clone()),
            (json!("44"), json!("webhook"))
        );
    }

    #[tokio::test]
    async fn start_external_node_adds_the_node_id() {
        let fx = Fixture::new().await;
        fx.node(
            json!([{ "id": "1", "stepType": "startExternalNode", "flowId": "44", "nodeId": "x9" }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        let job = &fx.jobs.send_flows()[0];
        assert_eq!(
            (job["flowId"].clone(), job["nodeId"].clone()),
            (json!("44"), json!("x9"))
        );
    }

    #[tokio::test]
    async fn split_traffic_starts_the_node_of_the_one_weighted_case() {
        let fx = Fixture::new().await;
        fx.node(
            json!([{ "id": "1", "stepType": "splitTraffic", "cases": [{ "value": 0 }, { "value": 100 }] }]),
            json!([{ "sourceHandle": "n1-case-0", "target": "n2" }, { "sourceHandle": "n1-case-1", "target": "n3" }]),
        )
        .await;
        fx.run(json!({ "metadata": { "type": "sequenceSchedule", "contactInboxId": "5", "sequenceStepId": "9", "sequenceId": "8", "dispatchId": "7" } }))
            .await
            .unwrap();
        let jobs = fx.jobs.send_flows();
        assert_eq!(jobs[0]["nodeId"], "n3");
        assert_eq!(jobs[0]["metadata"]["sequenceStepId"], "9");
    }
}

/// `pipeline-step.test.ts`: the action rules are the business tests
/// (`crates/business/tests/pipeline_flow_actions.rs`); here the step runs
/// on real rows and the outcome routes to the step states.
mod pipeline_step {
    use super::*;
    use chatbotx_core::flow_config::pipeline_step_default_fn;

    async fn board(fx: &Fixture) {
        fx.exec(
            r#"INSERT INTO "Pipeline" ("id", "workspaceId", "name", "type", "referenceName", "settings") VALUES (101, 2, 'Sales', 'opportunity', 'p_101', '{}');
               INSERT INTO "PipelineStage" ("id", "workspaceId", "pipelineId", "name", "position") VALUES (201, 2, 101, 'New', 0);"#,
        )
        .await;
    }

    async fn run(fx: &Fixture, action: &str, pipeline_id: &str) -> Option<Value> {
        let mut step =
            pipeline_step_default_fn(action, &json!({ "id": "1", "pipelineId": pipeline_id }));
        step["states"] =
            json!([{ "id": "skip", "stateType": "skip" }, { "id": "err", "stateType": "error" }]);
        fx.node(
            json!([step]),
            json!([{ "sourceHandle": "skip", "target": "n2" }, { "sourceHandle": "err", "target": "n3" }]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        fx.jobs.send_flows().last().map(|j| j["nodeId"].clone())
    }

    #[tokio::test]
    async fn add_to_pipeline_writes_the_item_and_succeeds() {
        let fx = Fixture::new().await;
        board(&fx).await;
        assert_eq!(run(&fx, "addToPipeline", "101").await, None);
        assert_eq!(
            fx.int(r#"SELECT count(*) FROM "PipelineItem" WHERE "contactId" = 3 AND "pipelineId" = 101"#).await,
            Some(1)
        );
    }

    #[tokio::test]
    async fn routes_skip_and_error_outcomes() {
        let fx = Fixture::new().await;
        board(&fx).await;
        // No open item for the contact: the move is skipped.
        assert_eq!(run(&fx, "moveToStage", "101").await, Some(json!("n2")));
        // An unknown pipeline is a caller-fixable error.
        assert_eq!(run(&fx, "addToPipeline", "999").await, Some(json!("n3")));
    }
}

/// `count-characters-step.test.ts`, `format-date-step.test.ts`,
/// `get-data-from-json-step.test.ts` and the `generateCode` step. Custom
/// fields 30 `bio` (text), 31 `len` (number), 32 `when` (datetime), 33
/// `pick` (text); Account Fields 40 `count` (number), 41 `src` (text, "abc"),
/// 42 `due` (datetime).
mod tool_steps {
    use super::*;

    async fn fields(fx: &Fixture, bio: Option<&'static str>) {
        fx.exec(
            r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES
                 (30, 'bio', 'shortText', 2), (31, 'len', 'number', 2), (32, 'when', 'datetime', 2), (33, 'pick', 'shortText', 2);
               INSERT INTO "BotField" ("id", "name", "type", "value", "workspaceId") VALUES
                 (40, 'count', 'number', NULL, 2), (41, 'src', 'shortText', 'abc', 2), (42, 'due', 'datetime', NULL, 2);"#,
        )
        .await;
        if let Some(bio) = bio {
            fx.sql(move |tx| {
                tx.execute(
                    r#"INSERT INTO "ContactCustomField" ("value", "contactId", "customFieldId") VALUES (?1, 3, 30)"#,
                    chatbotx_db::rusqlite::params![bio],
                )?;
                Ok(())
            })
            .await;
        }
    }

    async fn custom(fx: &Fixture, id: i64) -> Option<String> {
        fx.deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = 3 AND "customFieldId" = ?1"#,
                        [id],
                        |r| r.get(0),
                    )
                    .ok())
            })
            .await
            .unwrap()
    }

    async fn step(fx: &Fixture, step: Value) {
        let mut step = step;
        step["id"] = json!("1");
        step["states"] = json!([{ "id": "err", "stateType": "error" }]);
        fx.node(
            json!([step]),
            json!([{ "sourceHandle": "err", "target": "n3" }]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
    }

    #[tokio::test]
    async fn count_characters_writes_the_utf16_length_to_a_custom_field() {
        let fx = Fixture::new().await;
        fields(&fx, Some("héllo👍")).await;
        step(
            &fx,
            json!({ "stepType": "countCharacters", "inputFieldId": "30", "outputFieldId": "31" }),
        )
        .await;
        assert_eq!(custom(&fx, 31).await.as_deref(), Some("7"));
    }

    #[tokio::test]
    async fn count_characters_reads_and_writes_account_fields() {
        let fx = Fixture::new().await;
        fields(&fx, Some("hello")).await;
        step(&fx, json!({ "stepType": "countCharacters", "inputFieldId": "30", "outputFieldId": "bot_field:40" })).await;
        assert_eq!(
            fx.text(r#"SELECT "value" FROM "BotField" WHERE "id" = 40"#)
                .await
                .as_deref(),
            Some("5")
        );
        step(&fx, json!({ "stepType": "countCharacters", "inputFieldId": "bot_field:41", "outputFieldId": "31" })).await;
        assert_eq!(custom(&fx, 31).await.as_deref(), Some("3"));
    }

    #[tokio::test]
    async fn count_characters_skips_when_a_field_does_not_exist() {
        let fx = Fixture::new().await;
        fields(&fx, Some("hello")).await;
        step(
            &fx,
            json!({ "stepType": "countCharacters", "inputFieldId": "99", "outputFieldId": "31" }),
        )
        .await;
        step(&fx, json!({ "stepType": "countCharacters", "inputFieldId": "30", "outputFieldId": "bot_field:99" })).await;
        assert_eq!(custom(&fx, 31).await, None);
        assert!(fx.jobs.send_flows().is_empty());
    }

    #[tokio::test]
    async fn format_date_formats_in_the_workspace_zone_and_skips_temporal_outputs() {
        let fx = Fixture::new().await;
        fields(&fx, None).await;
        fx.exec(
            r#"UPDATE "Workspace" SET "timezone" = 'Europe/Berlin' WHERE "id" = 2;
               INSERT INTO "ContactCustomField" ("value", "contactId", "customFieldId") VALUES ('2026-01-02T03:04:05.000Z', 3, 32);"#,
        )
        .await;
        for timezone in ["contact", "workspace"] {
            step(&fx, json!({ "stepType": "formatDate", "inputFieldId": "32", "outputFieldId": "30", "format": "yyyy-MM-dd HH:mm", "timezone": timezone })).await;
            assert_eq!(
                custom(&fx, 30).await.as_deref(),
                Some("2026-01-02 04:04"),
                "{timezone}"
            );
        }
        // A temporal output (custom field or Account Field) is never written.
        step(&fx, json!({ "stepType": "formatDate", "inputFieldId": "32", "outputFieldId": "bot_field:42", "format": "yyyy", "timezone": "workspace" })).await;
        assert_eq!(
            fx.text(r#"SELECT "value" FROM "BotField" WHERE "id" = 42"#)
                .await,
            None
        );
        // No stored input value: nothing is written.
        step(&fx, json!({ "stepType": "formatDate", "inputFieldId": "33", "outputFieldId": "bot_field:40", "format": "yyyy", "timezone": "workspace" })).await;
        assert_eq!(
            fx.text(r#"SELECT "value" FROM "BotField" WHERE "id" = 40"#)
                .await,
            None
        );
    }

    #[tokio::test]
    async fn generate_code_writes_a_value_of_the_configured_kind() {
        let fx = Fixture::new().await;
        fields(&fx, None).await;
        step(&fx, json!({ "stepType": "generateCode", "type": "NUMERIC_VALUE", "min": 5, "max": 5, "outputFieldId": "30" })).await;
        assert_eq!(custom(&fx, 30).await.as_deref(), Some("5"));
        step(&fx, json!({ "stepType": "generateCode", "type": "NUMERIC_LENGTH", "min": 3, "max": 3, "outputFieldId": "30" })).await;
        let code = custom(&fx, 30).await.unwrap();
        assert!(
            code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit()),
            "{code}"
        );
        step(&fx, json!({ "stepType": "generateCode", "type": "ALPHANUMERIC_LENGTH", "min": 4, "max": 4, "outputFieldId": "bot_field:41" })).await;
        let code = fx
            .text(r#"SELECT "value" FROM "BotField" WHERE "id" = 41"#)
            .await
            .unwrap();
        assert!(
            code.len() == 4 && code.bytes().all(|b| b.is_ascii_alphabetic()),
            "{code}"
        );
    }

    #[tokio::test]
    async fn get_data_from_json_writes_custom_and_account_field_outputs() {
        let fx = Fixture::new().await;
        fields(&fx, Some(r#"{"a":{"b":1},"list":[{"x":"y"}],"n":null}"#)).await;
        step(
            &fx,
            json!({ "stepType": "getDataFromJson", "inputFieldId": "30", "mapping": [
                { "jsonPath": "a.b", "outputFieldId": "31" },
                { "jsonPath": "list.0.x", "outputFieldId": "33" },
                { "jsonPath": "n", "outputFieldId": "32" },
                { "jsonPath": "a", "outputFieldId": "99" },
                { "jsonPath": "a", "outputFieldId": "bot_field:41" },
            ] }),
        )
        .await;
        assert_eq!(custom(&fx, 31).await.as_deref(), Some("1"));
        assert_eq!(custom(&fx, 33).await.as_deref(), Some("y"));
        assert_eq!(custom(&fx, 32).await, None);
        assert_eq!(
            fx.text(r#"SELECT "value" FROM "BotField" WHERE "id" = 41"#)
                .await
                .as_deref(),
            Some(r#"{"b":1}"#)
        );
        assert!(fx.jobs.send_flows().is_empty());
    }

    #[tokio::test]
    async fn get_data_from_json_errors_without_input_or_with_invalid_json() {
        for bio in [None, Some("not json")] {
            let fx = Fixture::new().await;
            fields(&fx, bio).await;
            step(
                &fx,
                json!({ "stepType": "getDataFromJson", "inputFieldId": "30", "mapping": [] }),
            )
            .await;
            assert_eq!(fx.jobs.send_flows()[0]["nodeId"], "n3", "{bio:?}");
        }
    }
}

/// `copy-text-handler.test.ts` and `call-phone-number-handler.test.ts`: the
/// reply is the `chat` `sendFlowMessage` job's step.
mod reply_steps {
    use super::*;

    async fn reply(fx: &Fixture, step: Value) -> Vec<Value> {
        let mut step = step;
        step["id"] = json!("60");
        fx.node(json!([step]), json!([])).await;
        fx.run(json!({})).await.unwrap();
        fx.jobs
            .named("sendFlowMessage")
            .into_iter()
            .map(|j| j.data["data"]["step"].clone())
            .collect()
    }

    #[tokio::test]
    async fn copy_text_replies_with_the_trimmed_text_as_a_text_step() {
        let fx = Fixture::new().await;
        let long = "x".repeat(5000);
        assert_eq!(
            reply(&fx, json!({ "stepType": "copyText", "text": "  SAVE10  " })).await,
            vec![
                json!({ "id": "60", "nodeId": "n1", "stepType": "sendText", "text": "SAVE10", "buttons": [] })
            ]
        );
        // Still replies with a text too long for a native copy button.
        assert_eq!(
            reply(&fx, json!({ "stepType": "copyText", "text": long })).await[1]["text"],
            json!("x".repeat(5000))
        );
    }

    #[tokio::test]
    async fn copy_text_sends_nothing_when_the_text_resolves_to_nothing() {
        let fx = Fixture::new().await;
        assert!(
            reply(&fx, json!({ "stepType": "copyText", "text": "   " }))
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn call_phone_number_replies_with_the_normalised_number() {
        let fx = Fixture::new().await;
        assert_eq!(
            reply(
                &fx,
                json!({ "stepType": "callPhoneNumber", "phoneNumber": "+1 415 555 2671" })
            )
            .await,
            vec![
                json!({ "id": "60", "nodeId": "n1", "stepType": "sendText", "text": "+14155552671", "buttons": [] })
            ]
        );
    }

    #[tokio::test]
    async fn call_phone_number_sends_nothing_for_an_invalid_number() {
        let fx = Fixture::new().await;
        assert!(
            reply(
                &fx,
                json!({ "stepType": "callPhoneNumber", "phoneNumber": "call me" })
            )
            .await
            .is_empty()
        );
    }
}

/// `find-places-near-me-step.test.ts`: a Get User Data (location) step
/// before the find step stores the location in custom field 30.
mod find_places_near_me_step {
    use super::*;

    async fn setup(
        fx: &Fixture,
        channel: &'static str,
        location: Option<&'static str>,
        place: bool,
    ) {
        fx.sql(move |tx| {
            tx.execute(r#"UPDATE "ContactInbox" SET "channel" = ?1 WHERE "id" = 5"#, chatbotx_db::rusqlite::params![channel])?;
            tx.execute(r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (30, 'where', 'shortText', 2)"#, [])?;
            if let Some(location) = location {
                tx.execute(
                    r#"INSERT INTO "ContactCustomField" ("value", "contactId", "customFieldId") VALUES (?1, 3, 30)"#,
                    chatbotx_db::rusqlite::params![location],
                )?;
            }
            if place {
                tx.execute_batch(
                    r#"INSERT INTO "PlaceLocation" ("id", "workspaceId", "name", "latitude", "longitude") VALUES
                         (80, 2, 'Far', 48.13, 11.58), (81, 2, 'Near', 52.51, 13.39);"#,
                )?;
            }
            Ok(())
        })
        .await;
    }

    async fn run(fx: &Fixture, with_source: bool) -> Vec<Value> {
        let mut steps = vec![json!({ "id": "1", "stepType": "findPlacesNearMe", "nodeId": "n1" })];
        if with_source {
            steps.insert(0, json!({ "id": "g", "stepType": "getUserData", "replyFormat": "RF08", "outputFieldId": "30" }));
        }
        fx.node(json!(steps), json!([])).await;
        fx.run(json!({ "startFromStepId": "1", "commentAnchor": { "postId": "p" } }))
            .await
            .unwrap();
        fx.jobs
            .named("sendFlowMessage")
            .into_iter()
            .map(|j| j.data["data"].clone())
            .collect()
    }

    #[tokio::test]
    async fn sends_the_nearest_place_as_a_whatsapp_location_without_the_comment_anchor() {
        let fx = Fixture::new().await;
        setup(&fx, "whatsapp", Some("52.52,13.40"), true).await;
        let sent = run(&fx, true).await;
        assert_eq!(
            sent[0]["step"],
            json!({ "id": "1", "nodeId": "n1", "stepType": "whatsappLocation", "latitude": 52.51, "longitude": 13.39, "name": "Near" })
        );
        assert!(sent[0].get("commentAnchor").is_none());
    }

    #[tokio::test]
    async fn every_cannot_send_case_is_a_silent_skip() {
        for (channel, location, place, source) in [
            ("webchat", Some("52.52,13.40"), true, true),
            ("whatsapp", Some("52.52,13.40"), true, false),
            ("whatsapp", Some("not a place"), true, true),
            ("whatsapp", Some("52.52,13.40"), false, true),
        ] {
            let fx = Fixture::new().await;
            setup(&fx, channel, location, place).await;
            assert!(
                run(&fx, source).await.is_empty(),
                "{channel} {location:?} {place} {source}"
            );
        }
    }
}
