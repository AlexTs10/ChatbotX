//! Port of `packages/worker-config/__tests__/ai-file-job-ids.test.ts` (spec 5.1).

use chatbotx_jobs::queues::ai_file::{ai_file_embedding_job_id, ai_file_reindex_job_id};

/// Port of: packages/worker-config/__tests__/ai-file-job-ids.test.ts:10 "AI file job ids > each reindex run gets its own processing job id"
#[test]
fn each_reindex_run_gets_its_own_processing_job_id() {
    assert_eq!(
        ai_file_reindex_job_id("10", "run-a"),
        "heavy-ai-file-10-reindex-run-a"
    );
    assert_ne!(
        ai_file_reindex_job_id("10", "run-a"),
        ai_file_reindex_job_id("10", "run-b")
    );
}

/// Port of: packages/worker-config/__tests__/ai-file-job-ids.test.ts:19 "AI file job ids > embedding ids stay unchanged for jobs without a run"
#[test]
fn embedding_ids_stay_unchanged_for_jobs_without_a_run() {
    assert_eq!(
        ai_file_embedding_job_id("10", "20", None),
        "ai-file-embedding-10-20"
    );
}

/// Port of: packages/worker-config/__tests__/ai-file-job-ids.test.ts:25 "AI file job ids > embedding ids carry the run so a reindex re-embeds pending chunks"
#[test]
fn embedding_ids_carry_the_run_so_a_reindex_re_embeds_pending() {
    assert_eq!(
        ai_file_embedding_job_id("10", "20", Some("run-a")),
        "ai-file-embedding-10-20-rrun-a"
    );
}
