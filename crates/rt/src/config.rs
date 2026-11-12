//! Gateway configuration (`gateway-config.ts`): same env names and
//! defaults, without the Redis and builder URL keys.

use std::fmt;

use crate::protocol::PRESENCE_REPORT_INTERVAL_MS;

/// `PRESENCE_REPORT_COALESCE_MS`.
pub const PRESENCE_REPORT_COALESCE_MS: u64 = 1000;
/// The `{"hb":1}` interval.
pub const HEARTBEAT_INTERVAL_MS: u64 = 25_000;
/// The uWS `idleTimeout`: no inbound frame for this long closes the socket.
pub const IDLE_TIMEOUT_MS: u64 = 60_000;
/// Minimum length of `REALTIME_BROADCAST_SECRET` (decision D6).
pub const MIN_SECRET_LEN: usize = 32;

/// The gateway settings.
#[derive(Clone, PartialEq, Eq)]
pub struct RtConfig {
    /// `REALTIME_BROADCAST_SECRET` (HS256 key).
    pub secret: String,
    /// `REALTIME_CONNECTION_LIFETIME_MS` (default 1 800 000), ± 15 %.
    pub connection_lifetime_ms: u64,
    /// `REALTIME_MAX_CONNECTIONS` (members + guests, default 10 000).
    pub max_connections: usize,
    /// `REALTIME_MAX_CONNECTIONS_PER_WORKSPACE` (members, default 500).
    pub max_connections_per_workspace: usize,
    /// `REALTIME_MAX_GUEST_CONNECTIONS` (default 8 000).
    pub max_guest_connections: usize,
    /// `REALTIME_MAX_GUEST_CONNECTIONS_PER_WORKSPACE` (default 1 000).
    pub max_guest_connections_per_workspace: usize,
    /// `REALTIME_MAX_CONNECTIONS_PER_GUEST` (default 5).
    pub max_connections_per_guest: usize,
    /// `REALTIME_SHUTDOWN_DRAIN_MS` (default 5 000).
    pub shutdown_drain_ms: u64,
    /// Presence coalesce window (1 000 ms).
    pub presence_report_coalesce_ms: u64,
    /// Presence sweep interval (10 000 ms).
    pub presence_report_interval_ms: u64,
    /// Heartbeat interval (25 000 ms).
    pub heartbeat_interval_ms: u64,
    /// Idle timeout (60 000 ms).
    pub idle_timeout_ms: u64,
}

impl fmt::Debug for RtConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RtConfig")
            .field("secret", &"<redacted>")
            .field("connection_lifetime_ms", &self.connection_lifetime_ms)
            .field("max_connections", &self.max_connections)
            .field(
                "max_connections_per_workspace",
                &self.max_connections_per_workspace,
            )
            .field("max_guest_connections", &self.max_guest_connections)
            .field(
                "max_guest_connections_per_workspace",
                &self.max_guest_connections_per_workspace,
            )
            .field("max_connections_per_guest", &self.max_connections_per_guest)
            .field("shutdown_drain_ms", &self.shutdown_drain_ms)
            .field(
                "presence_report_coalesce_ms",
                &self.presence_report_coalesce_ms,
            )
            .field(
                "presence_report_interval_ms",
                &self.presence_report_interval_ms,
            )
            .field("heartbeat_interval_ms", &self.heartbeat_interval_ms)
            .field("idle_timeout_ms", &self.idle_timeout_ms)
            .finish()
    }
}

/// A variable that is missing or not valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtConfigError {
    pub var: &'static str,
    pub expected: &'static str,
}

impl fmt::Display for RtConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is not valid: expected {}", self.var, self.expected)
    }
}

impl std::error::Error for RtConfigError {}

impl RtConfig {
    /// The defaults with `secret` (no length check; see [`RtConfig::from_lookup`]).
    pub fn new(secret: impl Into<String>) -> RtConfig {
        RtConfig {
            secret: secret.into(),
            connection_lifetime_ms: 1_800_000,
            max_connections: 10_000,
            max_connections_per_workspace: 500,
            max_guest_connections: 8000,
            max_guest_connections_per_workspace: 1000,
            max_connections_per_guest: 5,
            shutdown_drain_ms: 5000,
            presence_report_coalesce_ms: PRESENCE_REPORT_COALESCE_MS,
            presence_report_interval_ms: PRESENCE_REPORT_INTERVAL_MS,
            heartbeat_interval_ms: HEARTBEAT_INTERVAL_MS,
            idle_timeout_ms: IDLE_TIMEOUT_MS,
        }
    }

    /// Reads the `REALTIME_*` variables with `get` (an empty value counts as
    /// unset). `REALTIME_BROADCAST_SECRET` is required, ≥ 32 chars.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<RtConfig, RtConfigError> {
        let var = |key: &str| get(key).filter(|value| !value.is_empty());
        let number = |key: &'static str, min: u64, default: u64| -> Result<u64, RtConfigError> {
            match var(key) {
                None => Ok(default),
                Some(value) => match value.trim().parse::<u64>() {
                    Ok(n) if n >= min => Ok(n),
                    _ => Err(RtConfigError {
                        var: key,
                        expected: "a whole number at or above its minimum",
                    }),
                },
            }
        };
        let count = |key: &'static str, default: u64| -> Result<usize, RtConfigError> {
            number(key, 1, default).map(|n| usize::try_from(n).unwrap_or(usize::MAX))
        };
        let secret = var("REALTIME_BROADCAST_SECRET")
            .filter(|secret| secret.chars().count() >= MIN_SECRET_LEN)
            .ok_or(RtConfigError {
                var: "REALTIME_BROADCAST_SECRET",
                expected: "a secret of 32 or more characters",
            })?;
        let defaults = RtConfig::new(secret);
        Ok(RtConfig {
            connection_lifetime_ms: number(
                "REALTIME_CONNECTION_LIFETIME_MS",
                60_000,
                defaults.connection_lifetime_ms,
            )?,
            max_connections: count("REALTIME_MAX_CONNECTIONS", 10_000)?,
            max_connections_per_workspace: count("REALTIME_MAX_CONNECTIONS_PER_WORKSPACE", 500)?,
            max_guest_connections: count("REALTIME_MAX_GUEST_CONNECTIONS", 8000)?,
            max_guest_connections_per_workspace: count(
                "REALTIME_MAX_GUEST_CONNECTIONS_PER_WORKSPACE",
                1000,
            )?,
            max_connections_per_guest: count("REALTIME_MAX_CONNECTIONS_PER_GUEST", 5)?,
            shutdown_drain_ms: number("REALTIME_SHUTDOWN_DRAIN_MS", 0, defaults.shutdown_drain_ms)?,
            ..defaults
        })
    }

    /// [`RtConfig::from_lookup`] on the process environment.
    pub fn from_env() -> Result<RtConfig, RtConfigError> {
        RtConfig::from_lookup(|key| std::env::var(key).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn load(vars: &[(&str, &str)]) -> Result<RtConfig, RtConfigError> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        RtConfig::from_lookup(|key| map.get(key).cloned())
    }

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn the_secret_is_required_and_at_least_32_chars() {
        assert_eq!(load(&[]).unwrap_err().var, "REALTIME_BROADCAST_SECRET");
        assert_eq!(
            load(&[("REALTIME_BROADCAST_SECRET", "short")])
                .unwrap_err()
                .var,
            "REALTIME_BROADCAST_SECRET"
        );
        assert_eq!(
            load(&[("REALTIME_BROADCAST_SECRET", SECRET)]).unwrap(),
            RtConfig::new(SECRET)
        );
    }

    #[test]
    fn values_and_minimums_follow_the_gateway_config() {
        let cfg = load(&[
            ("REALTIME_BROADCAST_SECRET", SECRET),
            ("REALTIME_CONNECTION_LIFETIME_MS", "60000"),
            ("REALTIME_MAX_CONNECTIONS", "7"),
            ("REALTIME_SHUTDOWN_DRAIN_MS", "0"),
        ])
        .unwrap();
        assert_eq!(cfg.connection_lifetime_ms, 60_000);
        assert_eq!(cfg.max_connections, 7);
        assert_eq!(cfg.shutdown_drain_ms, 0);
        assert_eq!(cfg.max_connections_per_guest, 5);
        assert!(
            load(&[
                ("REALTIME_BROADCAST_SECRET", SECRET),
                ("REALTIME_CONNECTION_LIFETIME_MS", "59999"),
            ])
            .is_err()
        );
        assert!(
            load(&[
                ("REALTIME_BROADCAST_SECRET", SECRET),
                ("REALTIME_MAX_CONNECTIONS", "0"),
            ])
            .is_err()
        );
    }
}
