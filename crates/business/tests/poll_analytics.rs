//! Port of `packages/business/__tests__/poll-analytics.test.ts`.

use chatbotx_business::poll::analytics::{AnalyticsCandidate, GenderVotes};
use chatbotx_business::poll::compute_poll_analytics;
use chatbotx_db::repo::poll::VoteBreakdownRow;

fn candidates() -> Vec<AnalyticsCandidate> {
    vec![
        AnalyticsCandidate {
            id: 1,
            name: "A".to_string(),
            image_url: None,
        },
        AnalyticsCandidate {
            id: 2,
            name: "B".to_string(),
            image_url: Some("u".to_string()),
        },
    ]
}

fn row(candidate_id: i64, gender: &str, votes: i64) -> VoteBreakdownRow {
    VoteBreakdownRow {
        candidate_id,
        gender: gender.to_string(),
        votes,
    }
}

#[test]
fn compute_poll_analytics_computes_percents_by_total_gender() {
    let result = compute_poll_analytics(
        &candidates(),
        &[row(1, "male", 1), row(1, "unknown", 1), row(2, "female", 2)],
    );
    assert_eq!(result.total_votes, 4);
    assert_eq!(
        result.gender,
        GenderVotes {
            male: 1,
            female: 2,
            unknown: 1
        }
    );
    let (c1, c2) = (&result.candidates[0], &result.candidates[1]);
    assert_eq!(
        (
            c1.total_votes,
            c1.total_percent,
            c1.male_votes,
            c1.male_percent,
            c1.female_percent
        ),
        (2, 50.0, 1, 100.0, 0.0)
    );
    assert_eq!(
        (c2.total_percent, c2.female_percent, c2.male_percent),
        (50.0, 100.0, 0.0)
    );
}

#[test]
fn compute_poll_analytics_rounds_to_one_decimal() {
    let result = compute_poll_analytics(&candidates(), &[row(1, "male", 1), row(2, "male", 2)]);
    assert_eq!(result.candidates[0].total_percent, 33.3);
    assert_eq!(result.candidates[1].total_percent, 66.7);
}

#[test]
fn compute_poll_analytics_empty_breakdown_gives_zeros() {
    let result = compute_poll_analytics(&candidates(), &[]);
    assert_eq!(result.total_votes, 0);
    for c in &result.candidates {
        assert_eq!(
            (
                c.total_percent,
                c.male_percent,
                c.female_percent,
                c.total_votes
            ),
            (0.0, 0.0, 0.0, 0)
        );
    }
}
