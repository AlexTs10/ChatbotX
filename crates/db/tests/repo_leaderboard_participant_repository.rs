//! Port of `packages/database/__tests__/leaderboard-participant-repository.test.ts` (P3.3).
//! The mocked `tx.execute` RETURNING rows become a real `LeaderboardParticipant` row.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo;
use chatbotx_db::repo::leaderboard_participant::{RankedRow, set_banned};
use common::{fresh_db, seed};
use repo_common::{ms, ok, setup};

const WS: i64 = 2001;
const CONTACT: i64 = 4001;

async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        ins!(tx, "Leaderboard", {
            "id" => 1_i64, "workspaceId" => WS, "name" => "Board", "startsAt" => 0_i64,
            "endsAt" => 1_i64, "platform" => "messenger", "scope" => "{}", "rules" => "[]",
            "tagIds" => "[]", "rewardSettings" => "{}",
        })?;
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/leaderboard-participant-repository.test.ts:20 "leaderboardParticipantRepository.setBanned > returns scoredAt as a Date, so unbanning can restore the ranking"
#[tokio::test]
async fn leaderboard_participant_repository_set_banned_returns_scored_at_as_a_date_so_unbanning_can_restore_the_ranking()
 {
    let db = seeded().await;
    let scored_at = ms("2026-10-15T03:00:00.123Z");
    setup(&db, move |tx| {
        ins!(tx, "LeaderboardParticipant", {
            "leaderboardId" => 1_i64, "contactId" => CONTACT, "workspaceId" => WS, "score" => 15_i64,
            "isBanned" => 1_i64, "scoredAt" => scored_at, "dailyCounts" => "{}", "breakdown" => "{}",
        })?;
        Ok(())
    })
    .await;
    let row = ok(repo::write(&db, |tx| set_banned(tx, 1, CONTACT, false)).await);
    assert_eq!(
        row,
        Some(RankedRow {
            contact_id: CONTACT,
            score: 15,
            scored_at,
        })
    );
}

/// Port of: packages/database/__tests__/leaderboard-participant-repository.test.ts:34 "leaderboardParticipantRepository.setBanned > returns null when the participant does not exist"
#[tokio::test]
async fn leaderboard_participant_repository_set_banned_returns_null_when_the_participant_does_not_exist()
 {
    let db = seeded().await;
    let row = ok(repo::write(&db, |tx| set_banned(tx, 1, CONTACT, true)).await);
    assert_eq!(row, None);
}
