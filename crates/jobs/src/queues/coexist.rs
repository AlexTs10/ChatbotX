//! Coexistence job ids (`queues/integration/coexist-job-ids.ts`).

/// `coexist-flush-v2-<phoneNumberId>`.
pub fn coexist_flush_job_id(phone_number_id: &str) -> String {
    todo!()
}

/// `coexist-run-<runId>-<attempts><suffix>`.
pub fn coexist_run_job_id(run_id: &str, attempts: u32, suffix: Option<&str>) -> String {
    todo!()
}

/// `coexist-run-<runId>-<attempts>-page-<pageNumber>`.
pub fn coexist_page_job_id(run_id: &str, attempts: u32, page_number: u32) -> String {
    todo!()
}

/// `-revive-<uuid v4>`: a new random suffix per call.
pub fn coexist_revive_job_suffix() -> String {
    todo!()
}
