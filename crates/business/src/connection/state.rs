//! `connection/state.ts`: the pure Connection status state machine.
//! ACTIVE = `connected | degraded` (quota held); INACTIVE = `needs_reauth |
//! paused | disconnected`. `quota_edge` is the one source of truth for the
//! consume / release edge.

use chatbotx_core::utils::connection::{
    ConnectionStatus as S, ConnectionStatusReason as R, is_active_connection_status,
    is_provider_offboarded_connection,
};

use crate::errors::BusinessError;

/// `ConnectionEvent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    ConnectCompleted,
    AuthSaved,
    RefreshTransientFailure,
    VerifyFailedNonAuth,
    VerifyOk,
    AuthRevoked,
    UserDisconnect,
    TeardownPause,
    TeardownResume,
    TeardownDisconnect,
    ProviderOffboarded,
    ProviderReconnected,
}

impl Event {
    /// The TS event name.
    pub fn as_str(self) -> &'static str {
        match self {
            Event::ConnectCompleted => "connect.completed",
            Event::AuthSaved => "auth.saved",
            Event::RefreshTransientFailure => "refresh.transient_failure",
            Event::VerifyFailedNonAuth => "verify.failed_non_auth",
            Event::VerifyOk => "verify.ok",
            Event::AuthRevoked => "auth.revoked",
            Event::UserDisconnect => "user.disconnect",
            Event::TeardownPause => "teardown.pause",
            Event::TeardownResume => "teardown.resume",
            Event::TeardownDisconnect => "teardown.disconnect",
            Event::ProviderOffboarded => "provider.offboarded",
            Event::ProviderReconnected => "provider.reconnected",
        }
    }
}

/// `"consume"` / `"release"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaEdge {
    Consume,
    Release,
}

/// `ConnectionTransitionResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub to: S,
    pub reason: Option<R>,
    pub quota_edge: Option<QuotaEdge>,
    pub noop: bool,
    pub held_by_provider_offboard: bool,
}

fn active(status: Option<S>) -> bool {
    status.is_some_and(is_active_connection_status)
}

fn result(from: Option<S>, to: S, reason: Option<R>) -> Transition {
    let quota_edge = match (active(from), is_active_connection_status(to)) {
        (false, true) => Some(QuotaEdge::Consume),
        (true, false) => Some(QuotaEdge::Release),
        _ => None,
    };
    Transition {
        to,
        reason,
        quota_edge,
        noop: from == Some(to),
        held_by_provider_offboard: false,
    }
}

/// `InvalidConnectionTransitionException` (`connectionInactive`, 409).
fn invalid(from: Option<S>, event: Event) -> BusinessError {
    BusinessError::exception(
        format!(
            "Connection cannot handle event \"{}\" from status \"{}\"",
            event.as_str(),
            from.map_or("∅", |s| s.as_str())
        ),
        "connectionInactive",
        409,
    )
}

/// `transitionConnection`. Errors only for `auth.saved`, `refresh.*`,
/// `verify.*` against an INACTIVE (or absent) status; every other unmodeled
/// combination is an idempotent no-op.
pub fn transition(
    from: Option<S>,
    from_reason: Option<R>,
    event: Event,
    reason: Option<R>,
) -> Result<Transition, BusinessError> {
    use Event as E;
    let offboarded = is_provider_offboarded_connection(from, from_reason);
    let checked = matches!(
        event,
        E::AuthSaved | E::RefreshTransientFailure | E::VerifyFailedNonAuth | E::VerifyOk
    );
    if checked && offboarded {
        return Ok(Transition {
            held_by_provider_offboard: true,
            ..result(from, S::Degraded, Some(R::ProviderOffboarded))
        });
    }
    if checked && !active(from) {
        return Err(invalid(from, event));
    }
    let same = |fallback: S| result(from, from.unwrap_or(fallback), None);
    Ok(match event {
        E::ConnectCompleted | E::AuthSaved | E::VerifyOk => result(from, S::Connected, None),
        E::RefreshTransientFailure | E::VerifyFailedNonAuth => {
            result(from, S::Degraded, Some(reason.unwrap_or(R::RefreshFailed)))
        }
        E::AuthRevoked if active(from) => result(
            from,
            S::NeedsReauth,
            Some(reason.unwrap_or(R::TokenRevoked)),
        ),
        E::AuthRevoked => same(S::NeedsReauth),
        E::UserDisconnect if from.is_none() => result(from, S::Disconnected, Some(R::Manual)),
        E::UserDisconnect | E::TeardownDisconnect if from == Some(S::Disconnected) => {
            same(S::Disconnected)
        }
        E::UserDisconnect => result(from, S::Disconnected, Some(reason.unwrap_or(R::Manual))),
        E::TeardownDisconnect => result(
            from,
            S::Disconnected,
            Some(reason.unwrap_or(R::WorkspacePurge)),
        ),
        E::TeardownPause if active(from) => {
            result(from, S::Paused, Some(reason.unwrap_or(R::TrialExpired)))
        }
        E::TeardownPause => same(S::Paused),
        E::TeardownResume if from == Some(S::Paused) => result(from, S::Connected, None),
        E::TeardownResume => same(S::Paused),
        E::ProviderOffboarded if active(from) => {
            result(from, S::Degraded, Some(R::ProviderOffboarded))
        }
        E::ProviderOffboarded => same(S::NeedsReauth),
        E::ProviderReconnected if offboarded => result(from, S::Connected, None),
        E::ProviderReconnected => same(S::NeedsReauth),
    })
}

/// Port of `connection/__tests__/state.test.ts`.
#[cfg(test)]
mod tests {
    use super::*;
    use Event as E;

    const INACTIVE: [S; 3] = [S::NeedsReauth, S::Paused, S::Disconnected];
    const ACTIVE: [S; 2] = [S::Connected, S::Degraded];

    fn t(from: Option<S>, event: Event) -> Transition {
        transition(from, None, event, None).expect("transition")
    }

    fn tr(from: Option<S>, from_reason: Option<R>, event: Event, reason: Option<R>) -> Transition {
        transition(from, from_reason, event, reason).expect("transition")
    }

    #[test]
    fn active_statuses() {
        assert!(is_active_connection_status(S::Connected));
        assert!(is_active_connection_status(S::Degraded));
        for s in INACTIVE {
            assert!(!is_active_connection_status(s));
        }
    }

    #[test]
    fn connect_completed() {
        for from in [None, Some(S::Disconnected), Some(S::NeedsReauth)] {
            let r = t(from, E::ConnectCompleted);
            assert_eq!(r.to, S::Connected);
            assert_eq!(r.quota_edge, Some(QuotaEdge::Consume));
        }
        let r = t(Some(S::Degraded), E::ConnectCompleted);
        assert_eq!((r.to, r.noop, r.quota_edge), (S::Connected, false, None));
        let r = t(Some(S::Connected), E::ConnectCompleted);
        assert_eq!((r.noop, r.quota_edge), (true, None));
        for from in [S::NeedsReauth, S::Paused] {
            let r = t(Some(from), E::ConnectCompleted);
            assert_eq!(
                (r.to, r.noop, r.quota_edge),
                (S::Connected, false, Some(QuotaEdge::Consume))
            );
        }
    }

    #[test]
    fn user_disconnect() {
        for from in ACTIVE {
            let r = t(Some(from), E::UserDisconnect);
            assert_eq!(
                (r.to, r.quota_edge),
                (S::Disconnected, Some(QuotaEdge::Release))
            );
        }
        for from in INACTIVE {
            assert_eq!(t(Some(from), E::UserDisconnect).quota_edge, None);
        }
        for from in [S::NeedsReauth, S::Paused] {
            let r = t(Some(from), E::UserDisconnect);
            assert_eq!((r.to, r.noop, r.quota_edge), (S::Disconnected, false, None));
        }
        for event in [E::UserDisconnect, E::TeardownDisconnect] {
            let r = tr(Some(S::Disconnected), None, event, Some(R::Manual));
            assert_eq!(
                (r.to, r.noop, r.reason, r.quota_edge),
                (S::Disconnected, true, None, None)
            );
        }
    }

    #[test]
    fn auth_revoked() {
        for from in ACTIVE {
            let r = t(Some(from), E::AuthRevoked);
            assert_eq!(
                (r.to, r.quota_edge, r.reason),
                (
                    S::NeedsReauth,
                    Some(QuotaEdge::Release),
                    Some(R::TokenRevoked)
                )
            );
        }
        for from in INACTIVE {
            let r = tr(Some(from), None, E::AuthRevoked, Some(R::TokenRevoked));
            assert_eq!(
                (r.to, r.quota_edge, r.noop, r.reason),
                (from, None, true, None)
            );
        }
    }

    #[test]
    fn refresh_and_verify() {
        for event in [E::RefreshTransientFailure, E::VerifyFailedNonAuth] {
            for from in ACTIVE {
                let r = t(Some(from), event);
                assert_eq!((r.to, r.quota_edge), (S::Degraded, None));
            }
        }
        for from in INACTIVE {
            for event in [
                E::RefreshTransientFailure,
                E::VerifyOk,
                E::VerifyFailedNonAuth,
            ] {
                assert!(transition(Some(from), None, event, None).is_err());
            }
        }
        for event in [E::AuthSaved, E::VerifyOk] {
            let r = t(Some(S::Degraded), event);
            assert_eq!((r.to, r.quota_edge), (S::Connected, None));
        }
    }

    #[test]
    fn teardown() {
        for from in ACTIVE {
            let r = t(Some(from), E::TeardownPause);
            assert_eq!((r.to, r.quota_edge), (S::Paused, Some(QuotaEdge::Release)));
            let r = t(Some(from), E::TeardownDisconnect);
            assert_eq!(
                (r.to, r.reason, r.quota_edge),
                (
                    S::Disconnected,
                    Some(R::WorkspacePurge),
                    Some(QuotaEdge::Release)
                )
            );
        }
        for from in INACTIVE {
            let r = tr(Some(from), None, E::TeardownPause, Some(R::TrialExpired));
            assert_eq!(
                (r.to, r.quota_edge, r.noop, r.reason),
                (from, None, true, None)
            );
            assert_eq!(t(Some(from), E::TeardownDisconnect).quota_edge, None);
        }
        let r = t(Some(S::Paused), E::TeardownDisconnect);
        assert_eq!((r.to, r.noop, r.quota_edge), (S::Disconnected, false, None));
        let r = t(Some(S::Paused), E::TeardownResume);
        assert_eq!(
            (r.to, r.quota_edge),
            (S::Connected, Some(QuotaEdge::Consume))
        );
        for from in [S::Connected, S::Degraded, S::NeedsReauth, S::Disconnected] {
            let r = t(Some(from), E::TeardownResume);
            assert_eq!((r.to, r.quota_edge), (from, None));
        }
    }

    #[test]
    fn full_table() {
        let events = [
            E::ConnectCompleted,
            E::AuthSaved,
            E::RefreshTransientFailure,
            E::VerifyFailedNonAuth,
            E::VerifyOk,
            E::AuthRevoked,
            E::UserDisconnect,
            E::TeardownPause,
            E::TeardownResume,
            E::TeardownDisconnect,
        ];
        let throws = [
            E::RefreshTransientFailure,
            E::VerifyFailedNonAuth,
            E::VerifyOk,
            E::AuthSaved,
        ];
        let statuses = [
            None,
            Some(S::Connected),
            Some(S::Degraded),
            Some(S::NeedsReauth),
            Some(S::Paused),
            Some(S::Disconnected),
        ];
        for from in statuses {
            for event in events {
                let err = transition(from, None, event, None).is_err();
                assert_eq!(
                    err,
                    !active(from) && throws.contains(&event),
                    "{from:?} {event:?}"
                );
            }
        }
    }

    #[test]
    fn provider_account_events() {
        for from in ACTIVE {
            let r = t(Some(from), E::ProviderOffboarded);
            assert_eq!(
                (r.to, r.reason, r.quota_edge),
                (S::Degraded, Some(R::ProviderOffboarded), None)
            );
        }
        for from in INACTIVE {
            let r = t(Some(from), E::ProviderOffboarded);
            assert_eq!((r.to, r.noop, r.quota_edge), (from, true, None));
        }
        let held = Some(R::ProviderOffboarded);
        let r = tr(Some(S::Degraded), held, E::ProviderOffboarded, None);
        assert_eq!((r.noop, r.reason), (true, held));
        let r = tr(Some(S::Degraded), held, E::ProviderReconnected, None);
        assert_eq!((r.to, r.reason, r.quota_edge), (S::Connected, None, None));
        for (from, reason) in [
            (S::Connected, None),
            (S::Degraded, Some(R::RefreshFailed)),
            (S::Paused, Some(R::TrialExpired)),
            (S::Disconnected, Some(R::Manual)),
            (S::NeedsReauth, Some(R::ProviderRevoked)),
        ] {
            let r = tr(Some(from), reason, E::ProviderReconnected, None);
            assert_eq!((r.to, r.noop, r.quota_edge), (from, true, None));
        }
        for event in [
            E::AuthSaved,
            E::VerifyOk,
            E::VerifyFailedNonAuth,
            E::RefreshTransientFailure,
        ] {
            let r = tr(Some(S::Degraded), held, event, Some(R::RefreshFailed));
            assert_eq!(
                (r.to, r.reason, r.noop, r.held_by_provider_offboard),
                (S::Degraded, held, true, true)
            );
        }
        let r = tr(Some(S::Degraded), held, E::ConnectCompleted, None);
        assert_eq!((r.to, r.reason), (S::Connected, None));
        let r = tr(
            Some(S::Degraded),
            held,
            E::AuthRevoked,
            Some(R::ProviderRevoked),
        );
        assert_eq!(
            (r.to, r.reason, r.quota_edge),
            (
                S::NeedsReauth,
                Some(R::ProviderRevoked),
                Some(QuotaEdge::Release)
            )
        );
    }
}
