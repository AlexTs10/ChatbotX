//! Port of `packages/redis/__tests__/distributed-lock.test.ts` (P2.7 spec
//! 3.3, T18-T20). `isLockAcquisitionError` -> `kv::is_lock_acquisition_error`
//! on `&(dyn Error + 'static)`. T19/T20 are TS-only (structural typing and
//! dynamic values); they follow the spec's Rust proposal: a wrapped lock
//! error is found through `source()`, and errors that only look like one are
//! rejected.

use std::error::Error;
use std::fmt;

use chatbotx_kv::{LockAcquisitionError, is_lock_acquisition_error};

/// A worker-style error that wraps the lock error (as fixtures wrap it).
#[derive(Debug)]
struct HandlerError {
    source: LockAcquisitionError,
}

impl fmt::Display for HandlerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("lock unavailable")
    }
}

impl Error for HandlerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// An error whose text names the lock error but is not one.
#[derive(Debug)]
struct MessageError(String);

impl fmt::Display for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for MessageError {}

/// A schedule-style error enum that is not a lock error (stands in for
/// `jobs::sequence::schedule::ScheduleError`; kv cannot depend on jobs).
#[derive(Debug)]
enum OtherScheduleError {
    Store(String),
}

impl fmt::Display for OtherScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OtherScheduleError::Store(message) => write!(f, "schedule store: {message}"),
        }
    }
}

impl Error for OtherScheduleError {}

/// Port of: packages/redis/__tests__/distributed-lock.test.ts:13 "isLockAcquisitionError > recognizes a redlock-universal error and scopes it to an optional key"
#[test]
fn recognizes_a_redlock_universal_error_and_scopes_it_to_an() {
    let error = LockAcquisitionError {
        key: "ingress:conv:1".to_owned(),
        attempts: 2,
    };

    assert!(is_lock_acquisition_error(&error, None));
    assert!(is_lock_acquisition_error(&error, Some("ingress:conv:1")));
    assert!(!is_lock_acquisition_error(
        &error,
        Some("msg:upsert:1:source-1")
    ));
}

/// Port of: packages/redis/__tests__/distributed-lock.test.ts:21 "isLockAcquisitionError > recognizes the structural error shape used by worker fixtures"
#[test]
fn recognizes_the_structural_error_shape_used_by_worker() {
    let error = HandlerError {
        source: LockAcquisitionError {
            key: "schedule:purge-workspaces".to_owned(),
            attempts: 1,
        },
    };

    assert!(is_lock_acquisition_error(&error, None));
    assert!(is_lock_acquisition_error(
        &error,
        Some("schedule:purge-workspaces")
    ));
    assert!(!is_lock_acquisition_error(&error, Some("schedule:other")));
}

/// Port of: packages/redis/__tests__/distributed-lock.test.ts:31 "isLockAcquisitionError > rejects incomplete lock-like errors and non-errors"
#[test]
fn rejects_incomplete_lock_like_errors_and_non_errors() {
    // TS: an Error named "LockAcquisitionError" without `code`.
    let lock_like = MessageError("LockAcquisitionError: missing code".to_owned());
    assert!(!is_lock_acquisition_error(&lock_like, None));
    // TS: an unrelated Error.
    let unrelated = std::io::Error::other("unrelated");
    assert!(!is_lock_acquisition_error(&unrelated, None));
    // TS: null / a string. No Rust value of that kind; an error enum that is
    // not a lock error.
    let other = OtherScheduleError::Store("lock unavailable".to_owned());
    assert!(!is_lock_acquisition_error(&other, None));
}
