//! Process configuration read from environment variables.

use std::fmt;
use std::path::PathBuf;

/// Settings for one server process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `HOST`, default `0.0.0.0`.
    pub host: String,
    /// `PORT`, default `3000`.
    pub port: u16,
    /// `DATA_DIR`, default `./data`.
    pub data_dir: PathBuf,
    /// `LOG_LEVEL`, default `info`. Accepts `tracing` filter directives.
    pub log_level: String,
    /// `CHATBOTX_WORKER_THREADS`, default `2`. Tokio worker threads.
    pub worker_threads: usize,
}

/// A variable that is set but has a value that is not valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub var: &'static str,
    pub value: String,
    pub expected: &'static str,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}={:?} is not valid: expected {}",
            self.var, self.value, self.expected
        )
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Read the configuration from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Read the configuration with `get`. An empty value counts as unset.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let var = |key: &str| get(key).filter(|value| !value.is_empty());

        let port = match var("PORT") {
            None => 3000,
            Some(value) => value.parse().map_err(|_| ConfigError {
                var: "PORT",
                value,
                expected: "a port number from 0 to 65535",
            })?,
        };
        let worker_threads = match var("CHATBOTX_WORKER_THREADS") {
            None => 2,
            Some(value) => match value.parse::<usize>() {
                Ok(n) if n > 0 => n,
                _ => {
                    return Err(ConfigError {
                        var: "CHATBOTX_WORKER_THREADS",
                        value,
                        expected: "a whole number of 1 or more",
                    });
                }
            },
        };

        Ok(Self {
            host: var("HOST").unwrap_or_else(|| "0.0.0.0".to_owned()),
            port,
            data_dir: var("DATA_DIR").map_or_else(|| PathBuf::from("./data"), PathBuf::from),
            log_level: var("LOG_LEVEL").unwrap_or_else(|| "info".to_owned()),
            worker_threads,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigError};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn load(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Config::from_lookup(|key| map.get(key).cloned())
    }

    #[test]
    fn defaults_when_nothing_is_set() {
        assert_eq!(
            load(&[]),
            Ok(Config {
                host: "0.0.0.0".into(),
                port: 3000,
                data_dir: PathBuf::from("./data"),
                log_level: "info".into(),
                worker_threads: 2,
            })
        );
    }

    #[test]
    fn empty_values_count_as_unset() {
        assert_eq!(load(&[("PORT", ""), ("HOST", "")]), load(&[]));
    }

    #[test]
    fn values_from_the_environment() {
        let config = load(&[
            ("HOST", "127.0.0.1"),
            ("PORT", "8080"),
            ("DATA_DIR", "/var/lib/chatbotx"),
            ("LOG_LEVEL", "debug,hyper=warn"),
            ("CHATBOTX_WORKER_THREADS", "4"),
        ])
        .expect("valid config");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert_eq!(config.data_dir, PathBuf::from("/var/lib/chatbotx"));
        assert_eq!(config.log_level, "debug,hyper=warn");
        assert_eq!(config.worker_threads, 4);
    }

    #[test]
    fn rejects_a_bad_port() {
        let err = load(&[("PORT", "70000")]).expect_err("port out of range");
        assert_eq!(err.var, "PORT");
        assert!(load(&[("PORT", "abc")]).is_err());
    }

    #[test]
    fn rejects_zero_worker_threads() {
        let err = load(&[("CHATBOTX_WORKER_THREADS", "0")]).expect_err("zero threads");
        assert_eq!(err.var, "CHATBOTX_WORKER_THREADS");
    }
}
