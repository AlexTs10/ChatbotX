//! `broadcastAnalyticsService.getStats` and
//! `sequenceAnalyticsService.getStepStats`: delivery counters of a
//! broadcast's recipients / a sequence step's dispatches. An unknown or
//! foreign id counts zeros.

use chatbotx_db::repo;
use chatbotx_db::rusqlite::{Connection, params_from_iter};

/// `BroadcastStats` / `SequenceStepStats` (`message:sent` = delivered +
/// failed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeliveryStats {
    pub delivered: i64,
    pub seen: i64,
    pub clicked: i64,
    pub failed: i64,
}

impl DeliveryStats {
    pub fn sent(&self) -> i64 {
        self.delivered + self.failed
    }
}

const COUNTS: &str =
    r#"COUNT("deliveredAt"), COUNT("seenAt"), COUNT("clickedAt"), COUNT("failedAt")"#;

fn stats(conn: &Connection, sql: &str, binds: &[i64]) -> repo::Result<DeliveryStats> {
    Ok(conn.query_row(sql, params_from_iter(binds), |row| {
        Ok(DeliveryStats {
            delivered: row.get(0)?,
            seen: row.get(1)?,
            clicked: row.get(2)?,
            failed: row.get(3)?,
        })
    })?)
}

/// `getStats` of one broadcast (recipients of a broadcast of the workspace).
pub fn broadcast_stats(
    conn: &Connection,
    workspace_id: i64,
    broadcast_id: i64,
) -> repo::Result<DeliveryStats> {
    stats(
        conn,
        &format!(
            r#"SELECT {COUNTS} FROM "ContactOnBroadcast" t
               WHERE t."broadcastId" = ?1
                 AND EXISTS (SELECT 1 FROM "Broadcast" b WHERE b."id" = t."broadcastId" AND b."workspaceId" = ?2)"#
        ),
        &[broadcast_id, workspace_id],
    )
}

/// `getStepStats` of one sequence step.
pub fn sequence_step_stats(
    conn: &Connection,
    workspace_id: i64,
    sequence_id: i64,
    step_id: i64,
) -> repo::Result<DeliveryStats> {
    stats(
        conn,
        &format!(
            r#"SELECT {COUNTS} FROM "SequenceDispatch"
               WHERE "workspaceId" = ?1 AND "sequenceId" = ?2 AND "stepId" = ?3"#
        ),
        &[workspace_id, sequence_id, step_id],
    )
}
