//! `poll/analytics.ts`.

use chatbotx_core::js::math_round;
use chatbotx_db::repo::poll::VoteBreakdownRow;

/// A candidate of [`PollAnalytics`].
#[derive(Debug, Clone, PartialEq)]
pub struct PollCandidateAnalytics {
    pub id: i64,
    pub name: String,
    pub image_url: Option<String>,
    pub total_votes: i64,
    pub total_percent: f64,
    pub male_votes: i64,
    pub male_percent: f64,
    pub female_votes: i64,
    pub female_percent: f64,
}

/// Votes by gender (`male`, `female`, `unknown`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenderVotes {
    pub male: i64,
    pub female: i64,
    pub unknown: i64,
}

impl GenderVotes {
    fn add(&mut self, gender: &str, votes: i64) {
        match gender {
            "male" => self.male += votes,
            "female" => self.female += votes,
            _ => self.unknown += votes,
        }
    }
    fn total(&self) -> i64 {
        self.male + self.female + self.unknown
    }
}

/// `PollAnalytics`.
#[derive(Debug, Clone, PartialEq)]
pub struct PollAnalytics {
    pub total_votes: i64,
    pub gender: GenderVotes,
    pub candidates: Vec<PollCandidateAnalytics>,
}

/// A candidate as `computePollAnalytics` reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalyticsCandidate {
    pub id: i64,
    pub name: String,
    pub image_url: Option<String>,
}

/// One decimal: `Math.round((part / whole) * 1000) / 10`; 0 when `whole` is 0.
fn percent(part: i64, whole: i64) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    math_round((part as f64 / whole as f64) * 1000.0) / 10.0
}

/// `computePollAnalytics`: totals per candidate and contact gender, with percents.
pub fn compute_poll_analytics(
    candidates: &[AnalyticsCandidate],
    breakdown: &[VoteBreakdownRow],
) -> PollAnalytics {
    let mut gender = GenderVotes::default();
    let mut per_candidate: Vec<(i64, GenderVotes)> = Vec::new();
    for row in breakdown {
        gender.add(&row.gender, row.votes);
        match per_candidate
            .iter_mut()
            .find(|(id, _)| *id == row.candidate_id)
        {
            Some((_, entry)) => entry.add(&row.gender, row.votes),
            None => {
                let mut entry = GenderVotes::default();
                entry.add(&row.gender, row.votes);
                per_candidate.push((row.candidate_id, entry));
            }
        }
    }
    let total_votes = gender.total();
    PollAnalytics {
        total_votes,
        gender,
        candidates: candidates
            .iter()
            .map(|candidate| {
                let votes = per_candidate
                    .iter()
                    .find(|(id, _)| *id == candidate.id)
                    .map(|(_, votes)| *votes)
                    .unwrap_or_default();
                let candidate_total = votes.total();
                PollCandidateAnalytics {
                    id: candidate.id,
                    name: candidate.name.clone(),
                    image_url: candidate.image_url.clone(),
                    total_votes: candidate_total,
                    total_percent: percent(candidate_total, total_votes),
                    male_votes: votes.male,
                    male_percent: percent(votes.male, gender.male),
                    female_votes: votes.female,
                    female_percent: percent(votes.female, gender.female),
                }
            })
            .collect(),
    }
}
