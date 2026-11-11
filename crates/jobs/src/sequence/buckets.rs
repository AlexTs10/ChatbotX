//! `apps/worker/src/sequence-scheduler/buckets.ts`: the assigned bucket
//! set (`SCHEDULER_BUCKET_RANGE`). With one process every bucket is
//! assigned by default (spec 3.2); the parse is kept for the env contract.

/// Env name.
pub const SCHEDULER_BUCKET_RANGE_ENV: &str = "SCHEDULER_BUCKET_RANGE";
/// Number of buckets (`calculateBucket` result is `0..BUCKET_COUNT`).
pub const BUCKET_COUNT: u32 = 256;

/// `getAssignedBuckets()` over the value of `SCHEDULER_BUCKET_RANGE`:
/// `None` → `0..=255`; `"a,b,c"` → that list; `"a-b"` → the inclusive range.
pub fn assigned_buckets(range: Option<&str>) -> Vec<u32> {
    todo!()
}
