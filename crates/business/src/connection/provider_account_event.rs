//! `providerAccountEventService.applyToTarget` + the provider half of
//! `connectionStateService` (`applyProviderAccountEvent`,
//! `markLegacyInboxUnhealthy`, `decideProviderEventOrder`): one Meta
//! `account_update` (revoke / offboard / reconnect) on one channel number.

use chatbotx_core::utils::connection::{
    ConnectionStatus, ConnectionStatusReason, ProviderAccountEventKind as Kind,
    connection_to_inbox_disconnect_reason, is_active_connection_status,
    is_provider_offboarded_connection,
};
use chatbotx_db::Db;
use chatbotx_db::repo::coexist_sync_run as runs;
use chatbotx_db::repo::integration_whatsapp::ProviderAccountTarget;
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};

use super::state::{Event, QuotaEdge, transition};
use crate::{BusinessError, Result, tx};

fn repo_err(error: impl Into<chatbotx_db::repo::Error>) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `PROVIDER_ACCOUNT_EVENT_STRATEGY[kind]`: `(event, reason, precedence, clearsLastError)`.
fn strategy(kind: Kind) -> (Event, Option<ConnectionStatusReason>, u8, bool) {
    match kind {
        Kind::Revoked => (
            Event::AuthRevoked,
            Some(ConnectionStatusReason::ProviderRevoked),
            3,
            false,
        ),
        Kind::Offboarded => (
            Event::ProviderOffboarded,
            Some(ConnectionStatusReason::ProviderOffboarded),
            1,
            false,
        ),
        Kind::Reconnected => (Event::ProviderReconnected, None, 2, true),
    }
}

/// `ProviderEventOrder`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Apply,
    Stale,
    Duplicate,
}

/// `decideProviderEventOrder`.
pub fn decide_order(stored: Option<(i64, Kind)>, at: i64, kind: Kind) -> Order {
    let Some((stored_at, stored_kind)) = stored else {
        return Order::Apply;
    };
    if at != stored_at {
        return if at > stored_at {
            Order::Apply
        } else {
            Order::Stale
        };
    }
    if kind == stored_kind {
        return Order::Duplicate;
    }
    if strategy(kind).2 > strategy(stored_kind).2 {
        Order::Apply
    } else {
        Order::Stale
    }
}

/// `ProviderAccountEventRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub integration_type: String,
    pub event: Kind,
    pub reason: String,
    pub initiated_by: String,
    pub occurred_at: i64,
}

/// `ProviderAccountEventOutcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Order(Order),
    LegacyRevoked,
    LegacyIgnored,
}

fn revoked_text(request: &Request) -> String {
    format!(
        "Provider removed access ({}, initiated by {})",
        request.reason, request.initiated_by
    )
}

/// `mirrorInboxStatus`.
fn mirror_inbox(
    conn: &Connection,
    inbox_id: i64,
    workspace_id: i64,
    to: ConnectionStatus,
    reason: Option<ConnectionStatusReason>,
    now: i64,
) -> Result<()> {
    if is_active_connection_status(to) {
        conn.execute(
            r#"UPDATE "Inbox" SET "status" = 'connected', "disconnectedAt" = NULL,
                   "disconnectReason" = NULL, "updatedAt" = ?3
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![inbox_id, workspace_id, now],
        )
        .map_err(repo_err)?;
        return Ok(());
    }
    match reason {
        Some(reason) => conn.execute(
            r#"UPDATE "Inbox" SET "status" = 'disconnected', "disconnectedAt" = ?3,
                   "disconnectReason" = ?4, "updatedAt" = ?3
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![
                inbox_id,
                workspace_id,
                now,
                connection_to_inbox_disconnect_reason(reason).as_str()
            ],
        ),
        None => conn.execute(
            r#"UPDATE "Inbox" SET "status" = 'disconnected', "updatedAt" = ?3
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![inbox_id, workspace_id, now],
        ),
    }
    .map_err(repo_err)?;
    Ok(())
}

/// The stored provider state of a connection.
struct Stored {
    workspace_id: i64,
    kind: String,
    inbox_id: Option<i64>,
    status: Option<ConnectionStatus>,
    reason: Option<ConnectionStatusReason>,
    event: Option<(i64, Kind)>,
    disconnected_at: Option<i64>,
}

fn stored(conn: &Connection, id: i64) -> Result<Option<Stored>> {
    conn.query_row(
        r#"SELECT "workspaceId", "kind", "inboxId", "status", "statusReason",
                  "providerEventAt", "providerEventKind", "disconnectedAt"
           FROM "Connection" WHERE "id" = ?1"#,
        [id],
        |r| {
            let at: Option<i64> = r.get(5)?;
            let kind: Option<String> = r.get(6)?;
            Ok(Stored {
                workspace_id: r.get(0)?,
                kind: r.get(1)?,
                inbox_id: r.get(2)?,
                status: r
                    .get::<_, String>(3)
                    .ok()
                    .and_then(|s| ConnectionStatus::parse(&s)),
                reason: r
                    .get::<_, Option<String>>(4)?
                    .and_then(|s| ConnectionStatusReason::parse(&s)),
                event: at.zip(kind.and_then(|k| Kind::parse(&k))),
                disconnected_at: r.get(7)?,
            })
        },
    )
    .optional()
    .map_err(repo_err)
}

/// `connectionStateService.applyProviderAccountEvent` + `transition` inside
/// the caller's transaction (the `channels` release of a revoke included).
fn apply_provider_event(
    conn: &Connection,
    connection_id: i64,
    request: &Request,
    owner_id: i64,
    last_error: Option<String>,
) -> Result<Order> {
    let Some(existing) = stored(conn, connection_id)? else {
        return Err(BusinessError::not_found(format!(
            "Connection {connection_id} not found"
        )));
    };
    let order = decide_order(existing.event, request.occurred_at, request.event);
    if order != Order::Apply {
        return Ok(order);
    }
    let (event, reason, _, clears) = strategy(request.event);
    let offboarded = is_provider_offboarded_connection(existing.status, existing.reason);
    let last_error: Option<Option<String>> = match last_error {
        Some(text) => Some(Some(text)),
        None if clears && offboarded => Some(None),
        None => None,
    };
    let result = transition(existing.status, existing.reason, event, reason)?;
    let now = now_ms();
    let mut sql = String::from(
        r#"UPDATE "Connection" SET "updatedAt" = ?1, "providerEventAt" = ?2, "providerEventKind" = ?3"#,
    );
    let last_error_value = last_error.clone().flatten();
    // A held event must not clear the error that explains the hold. (An
    // unused `?4` is fine: SQLite counts parameters by the highest index.)
    if last_error.is_some() && !(result.noop && result.held_by_provider_offboard) {
        sql.push_str(r#", "lastError" = ?4"#);
    }
    if result.noop {
        if let Some(reason) = result.reason {
            sql.push_str(&format!(r#", "statusReason" = '{}'"#, reason.as_str()));
        }
    } else {
        sql.push_str(&format!(
            r#", "status" = '{}', "statusReason" = {}"#,
            result.to.as_str(),
            result
                .reason
                .map_or("NULL".to_owned(), |r| format!("'{}'", r.as_str()))
        ));
        if result.quota_edge == Some(QuotaEdge::Consume) {
            sql.push_str(r#", "connectedAt" = ?1"#);
        }
        let disconnected_at = (result.to == ConnectionStatus::Disconnected)
            .then(|| existing.disconnected_at.unwrap_or(now));
        sql.push_str(&format!(
            r#", "disconnectedAt" = {}"#,
            disconnected_at.map_or("NULL".to_owned(), |t| t.to_string())
        ));
    }
    sql.push_str(r#" WHERE "id" = ?5"#);
    conn.execute(
        &sql,
        params![
            now,
            request.occurred_at,
            request.event.as_str(),
            last_error_value,
            connection_id
        ],
    )
    .map_err(repo_err)?;
    if let Some(inbox_id) = existing.inbox_id {
        mirror_inbox(
            conn,
            inbox_id,
            existing.workspace_id,
            result.to,
            if result.noop { None } else { result.reason },
            now,
        )?;
    }
    if existing.kind == "channel" {
        let metric = crate::user_quota::QuotaMetric::Channels;
        let usage = crate::workspace_usage::WorkspaceUsageMetric::Channels;
        match (result.noop, result.quota_edge) {
            (false, Some(QuotaEdge::Release)) => {
                crate::quota_enforcement::release_by(conn, owner_id, metric, 1)
                    .and_then(|()| {
                        crate::workspace_usage::adjust(conn, existing.workspace_id, usage, -1)
                    })
                    .unwrap_or_else(|error| {
                        tracing::warn!(%error, connection_id, "provider account event: channel quota release failed");
                    });
            }
            (false, Some(QuotaEdge::Consume)) => {
                crate::quota_enforcement::increment_by(conn, owner_id, metric, 1)
                    .and_then(|()| {
                        crate::workspace_usage::adjust(conn, existing.workspace_id, usage, 1)
                    })
                    .unwrap_or_else(|error| {
                        tracing::warn!(%error, connection_id, "provider account event: channel quota consume failed");
                    });
            }
            _ => {}
        }
    }
    Ok(order)
}

/// `stopCoexistSync`: a revoked account can no longer be synced.
fn stop_coexist(
    conn: &Connection,
    request: &Request,
    target: &ProviderAccountTarget,
) -> Result<()> {
    if !matches!(
        request.integration_type.as_str(),
        "whatsapp" | "messenger" | "instagram"
    ) {
        return Ok(());
    }
    let text = revoked_text(request);
    runs::set_integration_coexist_enabled(
        conn,
        &request.integration_type,
        target.workspace_id,
        target.integration_id,
        false,
        None,
    )?;
    runs::tear_down_active_runs_for_integration(
        conn,
        &request.integration_type,
        target.integration_id,
        &text,
    )?;
    Ok(())
}

/// `providerAccountEventService.applyToTarget`: the status change and a
/// revoke's coexist stop commit together; a stale event changes nothing.
pub async fn apply_to_target(
    db: &Db,
    request: Request,
    target: ProviderAccountTarget,
) -> Result<Outcome> {
    tx::write(db, move |conn| {
        let revoked = request.event == Kind::Revoked;
        let Some(connection_id) = target.connection_id else {
            if !revoked {
                tracing::warn!(
                    event = request.event.as_str(),
                    inbox_id = target.inbox_id,
                    "Provider account event skipped: no Connection row for a legacy channel"
                );
                return Ok(Outcome::LegacyIgnored);
            }
            stop_coexist(conn, &request, &target)?;
            // `markLegacyInboxUnhealthy` (`needs_reauth`, `provider_revoked`).
            mirror_inbox(
                conn,
                target.inbox_id,
                target.workspace_id,
                ConnectionStatus::NeedsReauth,
                Some(ConnectionStatusReason::ProviderRevoked),
                now_ms(),
            )?;
            return Ok(Outcome::LegacyRevoked);
        };
        conn.execute_batch("SAVEPOINT provider_event")
            .map_err(repo_err)?;
        let outcome = (|| {
            if revoked {
                stop_coexist(conn, &request, &target)?;
            }
            apply_provider_event(
                conn,
                connection_id,
                &request,
                target.workspace_owner_id,
                revoked.then(|| revoked_text(&request)),
            )
        })();
        let finish = match &outcome {
            Ok(Order::Apply) => "RELEASE provider_event",
            _ => "ROLLBACK TO provider_event; RELEASE provider_event",
        };
        conn.execute_batch(finish).map_err(repo_err)?;
        outcome.map(Outcome::Order)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_events_win_and_ties_follow_precedence() {
        assert_eq!(decide_order(None, 5, Kind::Revoked), Order::Apply);
        let stored = Some((10, Kind::Offboarded));
        assert_eq!(decide_order(stored, 11, Kind::Offboarded), Order::Apply);
        assert_eq!(decide_order(stored, 9, Kind::Revoked), Order::Stale);
        assert_eq!(decide_order(stored, 10, Kind::Offboarded), Order::Duplicate);
        assert_eq!(decide_order(stored, 10, Kind::Reconnected), Order::Apply);
        assert_eq!(
            decide_order(Some((10, Kind::Revoked)), 10, Kind::Reconnected),
            Order::Stale
        );
    }
}
