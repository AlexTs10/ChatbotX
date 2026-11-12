//! Port of `packages/flow-config/src/steps/change-gamification-score.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn gamification_score_operations = z::enum_(&["add", "subtract"]));

schema!(pub fn change_gamification_score_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("changeGamificationScore")).describe("Step type discriminator: \"changeGamificationScore\".")),
("leaderboardId", z::string().optional().describe("Leaderboard to score. Get ids from `leaderboards.list`.")),
("operation", gamification_score_operations().default(json!("add")).describe("\"add\" or \"subtract\" points.")),
("points", z::number().int().gte(1 as f64).lte(1000000 as f64).default(json!(10)).describe("How many points, 1-1,000,000.")),
("targetUser", z::boolean().default(json!(true)).describe("Change the chatting contact's score (User scores).")),
("targetReferrer", z::boolean().default(json!(false)).describe("Change the score of whoever referred this contact into the leaderboard (Referent scores)."))]));

default_fn!(pub fn change_gamification_score_step_default_fn = json!({"id": create_id(), "stepType": "changeGamificationScore", "operation": "add", "points": 10, "targetUser": true, "targetReferrer": false}));
