//! Worker DB fixtures for the ported step / runner tests: a migrated SQLite
//! file, a recording job queue, and one contact with a webchat inbox, a
//! conversation and a flow (user 1, workspace 2, contact 3, inbox 4, contact
//! inbox 5, conversation 6, flow 10, version 11).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chatbotx_db::Db;
use chatbotx_jobs::wait::WaitError;
use chatbotx_jobs::{
    AddResult, BoxFuture, Enqueuer, JobOptions, JobWaiter, JobsError, NewJob, QueueName,
};
use serde_json::{Value, json};

use crate::channel::RecordingChannel;
use crate::{Channels, Deps};

pub const CONTACT_INBOX: i64 = 5;
pub const CONVERSATION: i64 = 6;
pub const FLOW: i64 = 10;
pub const VERSION: i64 = 11;

/// One recorded `add`.
#[derive(Debug, Clone)]
pub struct Added {
    pub name: String,
    pub data: Value,
    pub opts: JobOptions,
}

#[derive(Default)]
pub struct Jobs(pub Mutex<Vec<Added>>, pub std::sync::atomic::AtomicBool);

impl Jobs {
    pub fn all(&self) -> Vec<Added> {
        self.0.lock().unwrap().clone()
    }

    /// The next `add` fails.
    pub fn fail_next(&self) {
        self.1.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The jobs named `name`.
    pub fn named(&self, name: &str) -> Vec<Added> {
        self.all().into_iter().filter(|j| j.name == name).collect()
    }

    /// The `data.data` of every `sendFlow` job.
    pub fn send_flows(&self) -> Vec<Value> {
        self.all()
            .into_iter()
            .filter(|j| j.name == "sendFlow")
            .map(|j| j.data["data"].clone())
            .collect()
    }
}

impl Enqueuer for Jobs {
    fn add(
        &self,
        _: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        if self.1.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return Box::pin(async { Err(JobsError::Store("queue down".into())) });
        }
        let mut jobs = self.0.lock().unwrap();
        jobs.push(Added {
            name: name.into(),
            data,
            opts,
        });
        let job_id = jobs.len().to_string();
        Box::pin(async move {
            Ok(AddResult {
                job_id,
                created: true,
            })
        })
    }

    fn add_bulk(
        &self,
        _: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        let mut added = self.0.lock().unwrap();
        let results = jobs
            .into_iter()
            .map(|job| {
                added.push(Added {
                    name: job.name,
                    data: job.data,
                    opts: job.opts,
                });
                AddResult {
                    job_id: added.len().to_string(),
                    created: true,
                }
            })
            .collect();
        Box::pin(async move { Ok(results) })
    }
}

/// Every waited chat job finishes at once.
pub struct Done;

impl JobWaiter for Done {
    fn wait_until_finished<'a>(
        &'a self,
        _: QueueName,
        _: &'a str,
        _: u64,
    ) -> BoxFuture<'a, Result<Value, WaitError>> {
        Box::pin(async { Ok(Value::Null) })
    }
}

/// A migrated DB with the base rows, its deps and the recorded jobs.
pub struct Fixture {
    pub deps: Deps,
    pub jobs: Arc<Jobs>,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    pub async fn new() -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "chatbotx-worker-steps-{}-{}",
            std::process::id(),
            chatbotx_db::repo::new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Arc::new(Db::open(dir.join("t.db")).unwrap());
        db.migrate().await.unwrap();
        db.write(|tx| {
            for sql in [
                r#"INSERT INTO "User" ("id", "email") VALUES (1, 'u@example.test')"#,
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (2, 'WS', 1)"#,
                r#"INSERT INTO "Contact" ("id", "firstName", "lastName", "workspaceId") VALUES (3, 'Ada', 'Lovelace', 2)"#,
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (4, 2, 'Web', 'webchat', 'w')"#,
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId") VALUES (5, 3, 3, 4, 'webchat', 'webchat', 's')"#,
                r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (6, 2, 3)"#,
                r#"INSERT INTO "Flow" ("id", "name", "workspaceId", "currentVersionId") VALUES (10, 'F', 2, 11)"#,
            ] {
                tx.execute(sql, [])?;
            }
            Ok(())
        })
        .await
        .unwrap();
        let jobs = Arc::new(Jobs::default());
        let deps = Deps {
            db,
            jobs: jobs.clone(),
            waiter: Arc::new(Done),
            realtime: None,
            channels: Channels::default(),
            app_url: "http://app.test".into(),
            variables: None,
            ai: Arc::new(crate::ai::Ai::live()),
            files: None,
        };
        Fixture { deps, jobs, dir }
    }

    /// Runs `f` in one write transaction.
    pub async fn sql<F>(&self, f: F)
    where
        F: FnOnce(&mut chatbotx_db::WriteTx<'_>) -> Result<(), chatbotx_db::DbError>
            + Send
            + 'static,
    {
        self.deps.db.write(f).await.unwrap();
    }

    pub async fn exec(&self, sql: &'static str) {
        self.sql(move |tx| {
            tx.execute_batch(sql)?;
            Ok(())
        })
        .await;
    }

    /// One integer of a query.
    pub async fn int(&self, sql: &'static str) -> Option<i64> {
        self.deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(sql, [], |r| r.get::<_, Option<i64>>(0))
                    .ok()
                    .flatten())
            })
            .await
            .unwrap()
    }

    pub async fn text(&self, sql: &'static str) -> Option<String> {
        self.deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(sql, [], |r| r.get::<_, Option<String>>(0))
                    .ok()
                    .flatten())
            })
            .await
            .unwrap()
    }

    /// Flow version 11 with `nodes` / `edges`.
    pub async fn flow(&self, nodes: Value, edges: Value) {
        let (nodes, edges) = (nodes.to_string(), edges.to_string());
        self.sql(move |tx| {
            tx.execute(
                r#"INSERT OR REPLACE INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "startNodeId")
                   VALUES (11, 2, 10, ?1, ?2, 0, 1)"#,
                chatbotx_db::rusqlite::params![nodes, edges],
            )?;
            Ok(())
        })
        .await;
    }

    /// A flow whose node `n1` holds `steps`, plus `edges`.
    pub async fn node(&self, steps: Value, edges: Value) {
        self.flow(
            json!([{ "id": "n1", "data": { "isStartNode": true, "details": { "steps": steps } } },
                   { "id": "n2", "data": { "details": { "steps": [] } } },
                   { "id": "n3", "data": { "details": { "steps": [] } } }]),
            edges,
        )
        .await;
    }

    /// Puts inbox 4 and contact inbox 5 on `channel` with its integration
    /// row; the channel is a [`RecordingChannel`].
    pub async fn on_channel(&mut self, channel: &'static str) -> Arc<RecordingChannel> {
        self.sql(move |tx| {
            tx.execute(
                r#"UPDATE "ContactInbox" SET "channel" = ?1, "source" = ?1 WHERE "id" = 5"#,
                chatbotx_db::rusqlite::params![channel],
            )?;
            tx.execute(
                r#"UPDATE "Inbox" SET "channel" = ?1, "markReadOnOutbound" = 1 WHERE "id" = 4"#,
                chatbotx_db::rusqlite::params![channel],
            )?;
            let row = match channel {
                "messenger" => r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId")
                                  VALUES (60, '{}', 'p', 'Page', '[]', '[]', '[]', 2, 4)"#,
                "instagram" => r#"INSERT INTO "IntegrationInstagram" ("id", "auth", "igId", "pageId", "name", "username", "conversationStarters", "persistentMenus", "workspaceId", "inboxId")
                                  VALUES (61, '{}', 'ig', 'p', 'IG', 'ig', '[]', '[]', 2, 4)"#,
                "whatsapp" => r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
                                 VALUES (62, '{}', 'pn', 'w', 'b', 'WA', 2, 4)"#,
                "telegram" => r#"INSERT INTO "IntegrationTelegram" ("id", "auth", "botId", "name", "workspaceId", "inboxId")
                                 VALUES (63, '{}', 'bot', 'TG', 2, 4)"#,
                other => panic!("no integration fixture for {other}"),
            };
            tx.execute(row, [])?;
            Ok(())
        })
        .await;
        let recording = RecordingChannel::new();
        self.deps.channels = Channels::default().with(channel, recording.clone());
        recording
    }

    /// `runFlowNode` for node `n1` with `extra` job fields.
    pub async fn run(&self, extra: Value) -> Result<(), chatbotx_jobs::JobError> {
        let mut data = json!({
            "conversationId": CONVERSATION.to_string(),
            "contactInboxId": CONTACT_INBOX.to_string(),
            "flowId": FLOW.to_string(),
            "flowVersionId": VERSION.to_string(),
            "nodeId": "n1",
        });
        for (k, v) in extra.as_object().cloned().unwrap_or_default() {
            data[k] = v;
        }
        crate::flow::run_flow_node(&self.deps, &data, "job-1:1".into()).await
    }
}
