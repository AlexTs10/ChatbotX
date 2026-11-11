//! Coexistence job ids (`queues/integration/coexist-job-ids.ts`).

/// `coexist-flush-v2-<phoneNumberId>`.
pub fn coexist_flush_job_id(phone_number_id: &str) -> String {
    format!("coexist-flush-v2-{phone_number_id}")
}

/// `coexist-run-<runId>-<attempts><suffix>`.
pub fn coexist_run_job_id(run_id: &str, attempts: u32, suffix: Option<&str>) -> String {
    format!("coexist-run-{run_id}-{attempts}{}", suffix.unwrap_or(""))
}

/// `coexist-run-<runId>-<attempts>-page-<pageNumber>`.
pub fn coexist_page_job_id(run_id: &str, attempts: u32, page_number: u32) -> String {
    format!("coexist-run-{run_id}-{attempts}-page-{page_number}")
}

/// `-revive-<uuid v4>`: a new random suffix per call.
pub fn coexist_revive_job_suffix() -> String {
    format!("-revive-{}", crate::rand::uuid_v4())
}
